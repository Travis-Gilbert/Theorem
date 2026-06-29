use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use mistralrs_core::{run_doctor, ModelDType, ModelSelected, MtpConfig, TokenSource};
use mistralrs_server_core::mistralrs_for_server_builder::{
    MistralRsForServerBuilder, ModelConfig as MistralServerModelConfig,
};
use mistralrs_server_core::mistralrs_server_router_builder::MistralRsServerRouterBuilder;
use serde::Serialize;

use crate::config::{LocalModelHostConfig, LocalModelTierConfig};
use crate::{LocalModelError, LocalModelResult};

#[derive(Debug, Serialize)]
pub struct LocalModelDoctorReport {
    pub endpoint: String,
    pub anthropic_messages_endpoint: String,
    pub health_endpoint: String,
    pub model_status_endpoint: String,
    pub default_model_id: String,
    pub quantized_model_id: String,
    pub quantized_filename: String,
    pub quantization: String,
    pub token_source: String,
    pub metal_requested: bool,
    pub cpu_only: bool,
    pub resident_memory_estimate_gb: f32,
    pub configured_tiers: Vec<LocalModelTierDoctor>,
    pub drafter: Option<LocalModelDrafterDoctor>,
    pub mistralrs: mistralrs_core::DoctorReport,
}

#[derive(Debug, Serialize)]
pub struct LocalModelTierDoctor {
    pub model_id: String,
    pub alias: Option<String>,
    pub quantized_model_id: String,
    pub quantized_filename: String,
    pub quantization: String,
    pub resident_memory_estimate_gb: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct LocalModelDrafterDoctor {
    pub model: String,
    pub n_predict: Option<usize>,
}

pub async fn serve(config: LocalModelHostConfig) -> LocalModelResult<()> {
    let bind = format!("{}:{}", config.host, config.port);
    let router = build_router(&config).await?;
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!(
        "[theorem-localmodel] serving local model host at http://{}",
        bind
    );
    eprintln!(
        "[theorem-localmodel] Anthropic Messages: http://{}/v1/messages",
        bind
    );
    eprintln!(
        "[theorem-localmodel] model status: http://{}/v1/models/status",
        bind
    );
    axum::serve(listener, router).await?;
    Ok(())
}

pub fn doctor_report(config: &LocalModelHostConfig) -> LocalModelDoctorReport {
    let base = format!("http://{}:{}", config.host, config.port);
    LocalModelDoctorReport {
        endpoint: base.clone(),
        anthropic_messages_endpoint: format!("{base}/v1/messages"),
        health_endpoint: format!("{base}/health"),
        model_status_endpoint: format!("{base}/v1/models/status"),
        default_model_id: config.api_model_id.clone(),
        quantized_model_id: config.quantized_model_id.clone(),
        quantized_filename: config.quantized_filename.clone(),
        quantization: config.quantization.clone(),
        token_source: config.token_source.clone(),
        metal_requested: !config.cpu,
        cpu_only: config.cpu,
        resident_memory_estimate_gb: config.resident_memory_estimate_gb,
        configured_tiers: config
            .tiers
            .iter()
            .chain(config.extra_models.iter())
            .map(tier_doctor)
            .collect(),
        drafter: config
            .drafter
            .as_ref()
            .map(|drafter| LocalModelDrafterDoctor {
                model: drafter.model.clone(),
                n_predict: drafter.n_predict,
            }),
        mistralrs: run_doctor(),
    }
}

async fn build_router(config: &LocalModelHostConfig) -> LocalModelResult<axum::Router> {
    preflight_primary_gguf(config)?;
    let mut builder = base_builder(config)?;
    if config.extra_models.is_empty() {
        builder = builder
            .with_model(model_selected(
                config.tok_model_id.clone(),
                config.quantized_model_id.clone(),
                config.quantized_filename.clone(),
                config.max_seq_len,
                config.max_batch_size,
            ))
            .with_model_id_override(config.api_model_id.clone());
    } else {
        let primary = apply_template_fields(
            MistralServerModelConfig::new(
                config.api_model_id.clone(),
                model_selected(
                    config.tok_model_id.clone(),
                    config.quantized_model_id.clone(),
                    config.quantized_filename.clone(),
                    config.max_seq_len,
                    config.max_batch_size,
                ),
            )
            .with_alias(config.api_model_id.clone()),
            config.chat_template.clone(),
            config.jinja_explicit.clone(),
            config.num_device_layers.clone(),
            config.in_situ_quant.clone(),
        );
        builder = builder
            .with_model_config(primary)
            .with_default_model_id(config.api_model_id.clone());
        for tier in &config.extra_models {
            builder = builder.with_model_config(tier_model_config(tier));
        }
    }

    let mistralrs = builder
        .build()
        .await
        .map_err(|error| LocalModelError::Model(error.to_string()))?;
    MistralRsServerRouterBuilder::new()
        .with_mistralrs(mistralrs)
        .build()
        .await
        .map_err(|error| LocalModelError::Model(error.to_string()))
}

fn preflight_primary_gguf(config: &LocalModelHostConfig) -> LocalModelResult<()> {
    let Some(path) = local_primary_gguf_path(config) else {
        return Ok(());
    };
    if !path.exists() {
        return Err(LocalModelError::Config(format!(
            "local_model GGUF file does not exist: {}",
            path.display()
        )));
    }
    let Some(architecture) = read_gguf_architecture(&path)? else {
        return Ok(());
    };
    if !is_mistralrs_supported_gguf_architecture(&architecture) {
        return Err(LocalModelError::Model(format!(
            "mistral.rs 15986c0 cannot load GGUF architecture `{architecture}` from {}; keep serving this GGUF with llama-server for now, or update the pinned mistral.rs revision once Gemma 4 GGUF support lands",
            path.display()
        )));
    }
    Ok(())
}

fn local_primary_gguf_path(config: &LocalModelHostConfig) -> Option<PathBuf> {
    let base = Path::new(&config.quantized_model_id);
    if !(base.exists() || base.is_absolute() || config.quantized_model_id.starts_with('.')) {
        return None;
    }
    let filename = config
        .quantized_filename
        .split(';')
        .next()
        .unwrap_or(config.quantized_filename.as_str());
    Some(base.join(filename))
}

fn is_mistralrs_supported_gguf_architecture(architecture: &str) -> bool {
    matches!(
        architecture.to_ascii_lowercase().as_str(),
        "llama"
            | "mpt"
            | "gptneox"
            | "gptj"
            | "gpt2"
            | "bloom"
            | "falcon"
            | "mamba"
            | "rwkv"
            | "phi2"
            | "phi3"
            | "starcoder2"
            | "qwen2"
            | "qwen3"
            | "qwen3moe"
            | "mistral3"
    )
}

fn read_gguf_architecture(path: &Path) -> LocalModelResult<Option<String>> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Ok(None);
    }
    let _version = read_u32(&mut file)?;
    let _tensor_count = read_u64(&mut file)?;
    let metadata_count = read_u64(&mut file)?;
    for _ in 0..metadata_count {
        let key = read_gguf_string(&mut file)?;
        let value_type = read_u32(&mut file)?;
        if key == "general.architecture" {
            if value_type != 8 {
                return Ok(None);
            }
            return Ok(Some(read_gguf_string(&mut file)?));
        }
        skip_gguf_value(&mut file, value_type)?;
    }
    Ok(None)
}

fn read_u32(reader: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> std::io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_gguf_string(reader: &mut impl Read) -> std::io::Result<String> {
    let len = read_u64(reader)? as usize;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()))
}

fn skip_gguf_value(reader: &mut (impl Read + Seek), value_type: u32) -> std::io::Result<()> {
    match value_type {
        0 | 1 | 7 => skip_bytes(reader, 1),
        2 | 3 => skip_bytes(reader, 2),
        4 | 5 | 6 => skip_bytes(reader, 4),
        8 => {
            let len = read_u64(reader)?;
            skip_bytes(reader, len)
        }
        9 => {
            let nested_type = read_u32(reader)?;
            let len = read_u64(reader)?;
            for _ in 0..len {
                skip_gguf_value(reader, nested_type)?;
            }
            Ok(())
        }
        10 | 11 | 12 => skip_bytes(reader, 8),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown GGUF metadata value type {other}"),
        )),
    }
}

fn skip_bytes(reader: &mut impl Seek, len: u64) -> std::io::Result<()> {
    reader.seek(std::io::SeekFrom::Current(len as i64))?;
    Ok(())
}

fn base_builder(config: &LocalModelHostConfig) -> LocalModelResult<MistralRsForServerBuilder> {
    let token_source = config
        .token_source
        .parse::<TokenSource>()
        .map_err(LocalModelError::Config)?;
    let drafter = config
        .drafter
        .as_ref()
        .map(|drafter| MtpConfig::new(drafter.model.clone(), drafter.n_predict));
    Ok(MistralRsForServerBuilder::new()
        .with_token_source(token_source)
        .with_cpu(config.cpu)
        .with_max_seqs(config.max_seqs)
        .set_paged_attn(config.paged_attention)
        .with_paged_ctxt_len_optional(config.paged_context_len)
        .with_paged_attn_gpu_mem_optional(config.paged_attention_gpu_mem_mb)
        .with_paged_attn_gpu_mem_usage_optional(config.paged_attention_gpu_mem_usage)
        .with_paged_attn_block_size_optional(config.paged_attention_block_size)
        .with_chat_template_optional(config.chat_template.clone())
        .with_jinja_explicit_optional(config.jinja_explicit.clone())
        .with_num_device_layers_optional(config.num_device_layers.clone())
        .with_in_situ_quant_optional(config.in_situ_quant.clone())
        .with_mtp_config_optional(drafter))
}

fn model_selected(
    tok_model_id: Option<String>,
    quantized_model_id: String,
    quantized_filename: String,
    max_seq_len: usize,
    max_batch_size: usize,
) -> ModelSelected {
    ModelSelected::GGUF {
        tok_model_id,
        quantized_model_id,
        quantized_filename,
        dtype: ModelDType::Auto,
        topology: None,
        max_seq_len,
        max_batch_size,
    }
}

fn tier_model_config(tier: &LocalModelTierConfig) -> MistralServerModelConfig {
    let api_id = tier.alias.clone().unwrap_or_else(|| tier.model_id.clone());
    apply_template_fields(
        MistralServerModelConfig::new(
            tier.model_id.clone(),
            model_selected(
                tier.tok_model_id.clone(),
                tier.quantized_model_id.clone(),
                tier.quantized_filename.clone(),
                tier.max_seq_len,
                tier.max_batch_size,
            ),
        )
        .with_alias(api_id),
        tier.chat_template.clone(),
        tier.jinja_explicit.clone(),
        None,
        tier.in_situ_quant.clone(),
    )
}

fn apply_template_fields(
    mut model: MistralServerModelConfig,
    chat_template: Option<String>,
    jinja_explicit: Option<String>,
    num_device_layers: Option<Vec<String>>,
    in_situ_quant: Option<String>,
) -> MistralServerModelConfig {
    if let Some(chat_template) = chat_template {
        model = model.with_chat_template(chat_template);
    }
    if let Some(jinja_explicit) = jinja_explicit {
        model = model.with_jinja_explicit(jinja_explicit);
    }
    if let Some(num_device_layers) = num_device_layers {
        model = model.with_num_device_layers(num_device_layers);
    }
    if let Some(in_situ_quant) = in_situ_quant {
        model = model.with_in_situ_quant(in_situ_quant);
    }
    model
}

fn tier_doctor(tier: &LocalModelTierConfig) -> LocalModelTierDoctor {
    LocalModelTierDoctor {
        model_id: tier.model_id.clone(),
        alias: tier.alias.clone(),
        quantized_model_id: tier.quantized_model_id.clone(),
        quantized_filename: tier.quantized_filename.clone(),
        quantization: tier.quantization.clone(),
        resident_memory_estimate_gb: tier.resident_memory_estimate_gb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gguf_architecture_metadata() {
        let path = write_fake_gguf("qwen3");
        let architecture = read_gguf_architecture(&path).unwrap();
        assert_eq!(architecture.as_deref(), Some("qwen3"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn preflight_rejects_unsupported_gemma4_gguf_before_upstream_panic() {
        let path = write_fake_gguf("gemma4");
        let dir = path.parent().unwrap().to_path_buf();
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        let mut config = LocalModelHostConfig::default();
        config.quantized_model_id = dir.to_string_lossy().to_string();
        config.quantized_filename = filename;

        let error = preflight_primary_gguf(&config).unwrap_err().to_string();
        assert!(error.contains("cannot load GGUF architecture `gemma4`"));
        assert!(error.contains("llama-server"));
        let _ = std::fs::remove_file(path);
    }

    fn write_fake_gguf(architecture: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "theorem-localmodel-{}-{}.gguf",
            architecture,
            std::process::id()
        ));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        write_gguf_string(&mut bytes, "general.architecture");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        write_gguf_string(&mut bytes, architecture);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn write_gguf_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}
