//! Multi-vector producers for ColPali-style late-interaction retrieval.
//!
//! Default builds keep this module dependency-light: callers get a stable
//! producer contract, deterministic fixtures, and hot/cold projection helpers.
//! The real Candle/ColPali model load is compiled only with `colpali-candle`.

#[cfg(feature = "colpali-candle")]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use rustyred_thg_core::{ThgError, ThgResult};

use crate::multivector::{
    quantize_sign_bits, BinaryMultiVectorSet, MultiVectorEmbeddingSet, MultiVectorManifest,
};

pub const DEFAULT_COLPALI_EMBEDDING_DIM: usize = 128;
pub const DEFAULT_COLPALI_MODEL_ID: &str = "vidore/colpali-v1.2-merged";
pub const DEFAULT_COLPALI_TOKENIZER_MODEL_ID: &str = "vidore/colpali";
pub const DEFAULT_COLPALI_REVISION: &str = "main";
pub const DEFAULT_COLPALI_DUMMY_IMAGE_PROMPT: &str = "Describe the image";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MultiVectorEncodeRequest {
    pub embedding_set_id: String,
    pub content_id: String,
    pub segments: Vec<MultiVectorInputSegment>,
    pub max_vectors: Option<usize>,
}

impl MultiVectorEncodeRequest {
    pub fn text(
        embedding_set_id: impl Into<String>,
        content_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            embedding_set_id: embedding_set_id.into(),
            content_id: content_id.into(),
            segments: vec![MultiVectorInputSegment::Text {
                segment_id: "text:0".to_string(),
                text: text.into(),
            }],
            max_vectors: None,
        }
    }

    pub fn image_rgb8(
        embedding_set_id: impl Into<String>,
        content_id: impl Into<String>,
        segment_id: impl Into<String>,
        width: usize,
        height: usize,
        rgb: Vec<u8>,
    ) -> Self {
        Self {
            embedding_set_id: embedding_set_id.into(),
            content_id: content_id.into(),
            segments: vec![MultiVectorInputSegment::ImageRgb8 {
                segment_id: segment_id.into(),
                width,
                height,
                rgb,
            }],
            max_vectors: None,
        }
    }

    fn validate(&self) -> ThgResult<()> {
        if self.embedding_set_id.trim().is_empty() {
            return Err(ThgError::new(
                "multivector_encode_invalid_request",
                "embedding_set_id must not be empty",
            ));
        }
        if self.content_id.trim().is_empty() {
            return Err(ThgError::new(
                "multivector_encode_invalid_request",
                "content_id must not be empty",
            ));
        }
        if self.segments.is_empty() {
            return Err(ThgError::new(
                "multivector_encode_empty_segments",
                "encode request must contain at least one segment",
            ));
        }
        for (idx, segment) in self.segments.iter().enumerate() {
            segment.validate(idx)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MultiVectorInputSegment {
    Text {
        segment_id: String,
        text: String,
    },
    ImageRgb8 {
        segment_id: String,
        width: usize,
        height: usize,
        rgb: Vec<u8>,
    },
}

impl MultiVectorInputSegment {
    fn validate(&self, idx: usize) -> ThgResult<()> {
        match self {
            Self::Text { segment_id, text } => {
                if segment_id.trim().is_empty() || text.trim().is_empty() {
                    return Err(ThgError::new(
                        "multivector_encode_invalid_segment",
                        format!("text segment {idx} must have a non-empty id and text"),
                    ));
                }
            }
            Self::ImageRgb8 {
                segment_id,
                width,
                height,
                rgb,
            } => {
                if segment_id.trim().is_empty() || *width == 0 || *height == 0 {
                    return Err(ThgError::new(
                        "multivector_encode_invalid_segment",
                        format!("image segment {idx} must have a non-empty id and size"),
                    ));
                }
                let expected_len = width.saturating_mul(*height).saturating_mul(3);
                if rgb.len() != expected_len {
                    return Err(ThgError::new(
                        "multivector_encode_invalid_segment",
                        format!(
                            "image segment {idx} has {} RGB bytes, expected {expected_len}",
                            rgb.len()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

pub trait MultiVectorProducer {
    fn encode(&mut self, request: &MultiVectorEncodeRequest) -> ThgResult<MultiVectorEmbeddingSet>;
    fn model_id(&self) -> &str;
    fn model_version(&self) -> &str;
    fn producer_id(&self) -> &str;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashingMultiVectorProducer {
    dimension: usize,
    model_id: String,
    model_version: String,
}

impl HashingMultiVectorProducer {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension: dimension.max(1),
            model_id: "colpali-hash-fixture".to_string(),
            model_version: "test-v1".to_string(),
        }
    }

    pub fn with_model(
        dimension: usize,
        model_id: impl Into<String>,
        model_version: impl Into<String>,
    ) -> Self {
        Self {
            dimension: dimension.max(1),
            model_id: model_id.into(),
            model_version: model_version.into(),
        }
    }
}

impl Default for HashingMultiVectorProducer {
    fn default() -> Self {
        Self::new(DEFAULT_COLPALI_EMBEDDING_DIM)
    }
}

impl MultiVectorProducer for HashingMultiVectorProducer {
    fn encode(&mut self, request: &MultiVectorEncodeRequest) -> ThgResult<MultiVectorEmbeddingSet> {
        request.validate()?;
        let mut vectors = Vec::new();
        for segment in &request.segments {
            match segment {
                MultiVectorInputSegment::Text { segment_id, text } => {
                    for token in tokenize_text(text) {
                        push_if_within_budget(
                            &mut vectors,
                            request.max_vectors,
                            hash_embedding(
                                format!("text:{segment_id}:{token}").as_bytes(),
                                self.dimension,
                            ),
                        );
                    }
                }
                MultiVectorInputSegment::ImageRgb8 {
                    segment_id,
                    width,
                    height,
                    rgb,
                } => {
                    for (chunk_idx, chunk) in
                        rgb.chunks(image_chunk_size(*width, *height)).enumerate()
                    {
                        let mut key =
                            format!("image:{segment_id}:{width}x{height}:{chunk_idx}").into_bytes();
                        key.extend_from_slice(chunk);
                        push_if_within_budget(
                            &mut vectors,
                            request.max_vectors,
                            hash_embedding(&key, self.dimension),
                        );
                    }
                }
            }
            if budget_exhausted(vectors.len(), request.max_vectors) {
                break;
            }
        }
        if vectors.is_empty() {
            return Err(ThgError::new(
                "multivector_encode_empty_output",
                "producer emitted no vectors",
            ));
        }
        let set = MultiVectorEmbeddingSet {
            embedding_set_id: request.embedding_set_id.clone(),
            content_id: request.content_id.clone(),
            model_id: self.model_id.clone(),
            model_version: self.model_version.clone(),
            vectors,
        };
        set.dim()?;
        Ok(set)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn model_version(&self) -> &str {
        &self.model_version
    }

    fn producer_id(&self) -> &str {
        "hashing_multivector"
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MultiVectorProjectionBundle {
    pub manifest: MultiVectorManifest,
    pub binary_projection: BinaryMultiVectorSet,
}

pub fn project_multivector_tiers(
    set: &MultiVectorEmbeddingSet,
    exact_object_ref: Option<String>,
    binary_projection_ref: Option<String>,
) -> ThgResult<MultiVectorProjectionBundle> {
    let binary_projection = quantize_sign_bits(set)?;
    let manifest =
        MultiVectorManifest::from_exact_set(set, exact_object_ref, binary_projection_ref)?;
    Ok(MultiVectorProjectionBundle {
        manifest,
        binary_projection,
    })
}

fn tokenize_text(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
}

fn image_chunk_size(width: usize, height: usize) -> usize {
    let pixel_bytes = width.saturating_mul(height).saturating_mul(3).max(1);
    (pixel_bytes / 64).clamp(192, 4096)
}

fn push_if_within_budget(
    vectors: &mut Vec<Vec<f32>>,
    max_vectors: Option<usize>,
    vector: Vec<f32>,
) {
    if !budget_exhausted(vectors.len(), max_vectors) {
        vectors.push(vector);
    }
}

fn budget_exhausted(vector_count: usize, max_vectors: Option<usize>) -> bool {
    max_vectors.is_some_and(|max_vectors| vector_count >= max_vectors)
}

fn hash_embedding(key: &[u8], dimension: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; dimension.max(1)];
    for (idx, slot) in vector.iter_mut().enumerate() {
        let mut bytes = Vec::with_capacity(key.len() + 8);
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&(idx as u64).to_le_bytes());
        let hash = fnv1a(&bytes);
        let scaled = ((hash >> 11) as f64) / ((1_u64 << 53) as f64);
        *slot = (scaled as f32 * 2.0) - 1.0;
    }
    l2_normalize(&mut vector);
    vector
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn l2_normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    } else if let Some(first) = vector.first_mut() {
        *first = 1.0;
    }
}

#[cfg(feature = "colpali-candle")]
pub struct CandleColPaliProducer {
    model: candle_transformers::models::colpali::Model,
    config: candle_transformers::models::paligemma::Config,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    dtype: candle_core::DType,
    model_id: String,
    model_version: String,
    dummy_image_prompt: String,
}

#[cfg(feature = "colpali-candle")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandleColPaliConfig {
    pub model_id: String,
    pub revision: String,
    pub tokenizer_model_id: String,
    pub tokenizer_revision: String,
    pub tokenizer_file: Option<PathBuf>,
    pub weight_files: Option<Vec<PathBuf>>,
    pub cpu: bool,
    pub dummy_image_prompt: String,
}

#[cfg(feature = "colpali-candle")]
impl Default for CandleColPaliConfig {
    fn default() -> Self {
        Self {
            model_id: DEFAULT_COLPALI_MODEL_ID.to_string(),
            revision: DEFAULT_COLPALI_REVISION.to_string(),
            tokenizer_model_id: DEFAULT_COLPALI_TOKENIZER_MODEL_ID.to_string(),
            tokenizer_revision: DEFAULT_COLPALI_REVISION.to_string(),
            tokenizer_file: None,
            weight_files: None,
            cpu: true,
            dummy_image_prompt: DEFAULT_COLPALI_DUMMY_IMAGE_PROMPT.to_string(),
        }
    }
}

#[cfg(feature = "colpali-candle")]
impl CandleColPaliProducer {
    pub fn new(config: CandleColPaliConfig) -> ThgResult<Self> {
        use candle_nn::VarBuilder;
        use candle_transformers::models::{colpali, paligemma};
        use hf_hub::api::sync::Api;
        use hf_hub::{Repo, RepoType};

        let api = Api::new().map_err(model_error)?;
        let model_repo = api.repo(Repo::with_revision(
            config.model_id.clone(),
            RepoType::Model,
            config.revision.clone(),
        ));
        let tokenizer_file = match config.tokenizer_file {
            Some(path) => path,
            None => api
                .repo(Repo::with_revision(
                    config.tokenizer_model_id.clone(),
                    RepoType::Model,
                    config.tokenizer_revision.clone(),
                ))
                .get("tokenizer.json")
                .map_err(model_error)?,
        };
        let weight_files = match config.weight_files {
            Some(paths) if !paths.is_empty() => paths,
            _ => hub_load_safetensors(&model_repo, "model.safetensors.index.json")?,
        };
        let device = select_candle_device(config.cpu)?;
        let dtype = if device.is_cuda() {
            candle_core::DType::BF16
        } else {
            candle_core::DType::F32
        };
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_file).map_err(model_error)?;
        let model_config = paligemma::Config::paligemma_3b_448();
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_files, dtype, &device)
                .map_err(model_error)?
        };
        let model = colpali::Model::new(&model_config, vb).map_err(model_error)?;
        Ok(Self {
            model,
            config: model_config,
            tokenizer,
            device,
            dtype,
            model_id: config.model_id,
            model_version: config.revision,
            dummy_image_prompt: config.dummy_image_prompt,
        })
    }

    fn encode_text_segment(
        &mut self,
        segment_id: &str,
        text: &str,
        max_vectors: Option<usize>,
        out: &mut Vec<Vec<f32>>,
    ) -> ThgResult<()> {
        let input = self.tokenize_one(text)?;
        let embeddings = self.model.forward_text(&input).map_err(model_error)?;
        append_tensor_vectors(
            segment_id,
            embeddings,
            max_vectors,
            out,
            "colpali_text_embedding_shape",
        )
    }

    fn encode_image_segment(
        &mut self,
        segment_id: &str,
        width: usize,
        height: usize,
        rgb: &[u8],
        max_vectors: Option<usize>,
        out: &mut Vec<Vec<f32>>,
    ) -> ThgResult<()> {
        let image = image_tensor_from_rgb8(
            width,
            height,
            rgb,
            self.config.vision_config.image_size,
            &self.device,
            self.dtype,
        )?;
        let dummy_input = self.tokenize_one(&self.dummy_image_prompt)?;
        let embeddings = self
            .model
            .forward_images(&image, &dummy_input)
            .map_err(model_error)?;
        append_tensor_vectors(
            segment_id,
            embeddings,
            max_vectors,
            out,
            "colpali_image_embedding_shape",
        )
    }

    fn tokenize_one(&self, text: &str) -> ThgResult<candle_core::Tensor> {
        let tokens = self.tokenizer.encode(text, true).map_err(model_error)?;
        let ids = tokens.get_ids().to_vec();
        candle_core::Tensor::new(ids.as_slice(), &self.device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(model_error)
    }
}

#[cfg(feature = "colpali-candle")]
impl MultiVectorProducer for CandleColPaliProducer {
    fn encode(&mut self, request: &MultiVectorEncodeRequest) -> ThgResult<MultiVectorEmbeddingSet> {
        request.validate()?;
        let mut vectors = Vec::new();
        for segment in &request.segments {
            match segment {
                MultiVectorInputSegment::Text { segment_id, text } => {
                    self.encode_text_segment(segment_id, text, request.max_vectors, &mut vectors)?
                }
                MultiVectorInputSegment::ImageRgb8 {
                    segment_id,
                    width,
                    height,
                    rgb,
                } => self.encode_image_segment(
                    segment_id,
                    *width,
                    *height,
                    rgb,
                    request.max_vectors,
                    &mut vectors,
                )?,
            }
            if budget_exhausted(vectors.len(), request.max_vectors) {
                break;
            }
        }
        if vectors.is_empty() {
            return Err(ThgError::new(
                "multivector_encode_empty_output",
                "ColPali emitted no vectors",
            ));
        }
        let set = MultiVectorEmbeddingSet {
            embedding_set_id: request.embedding_set_id.clone(),
            content_id: request.content_id.clone(),
            model_id: self.model_id.clone(),
            model_version: self.model_version.clone(),
            vectors,
        };
        set.dim()?;
        Ok(set)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn model_version(&self) -> &str {
        &self.model_version
    }

    fn producer_id(&self) -> &str {
        "candle_colpali"
    }
}

#[cfg(feature = "colpali-candle")]
fn select_candle_device(cpu: bool) -> ThgResult<candle_core::Device> {
    if !cpu {
        return Err(ThgError::new(
            "colpali_device",
            "colpali-candle is CPU-only until GPU feature wiring is benchmarked",
        ));
    }
    Ok(candle_core::Device::Cpu)
}

#[cfg(feature = "colpali-candle")]
fn hub_load_safetensors(
    repo: &hf_hub::api::sync::ApiRepo,
    json_file: &str,
) -> ThgResult<Vec<PathBuf>> {
    use std::collections::BTreeSet;

    let json_file_path = repo.get(json_file).map_err(model_error)?;
    let json_file = std::fs::File::open(&json_file_path).map_err(model_error)?;
    let json: serde_json::Value = serde_json::from_reader(json_file).map_err(model_error)?;
    let weight_map = json
        .get("weight_map")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            ThgError::new(
                "colpali_model_load",
                format!("no weight_map object in {}", json_file_path.display()),
            )
        })?;
    let mut safetensors_files = BTreeSet::new();
    for value in weight_map.values() {
        if let Some(file) = value.as_str() {
            safetensors_files.insert(file.to_string());
        }
    }
    safetensors_files
        .into_iter()
        .map(|file| repo.get(&file).map_err(model_error))
        .collect()
}

#[cfg(feature = "colpali-candle")]
fn image_tensor_from_rgb8(
    width: usize,
    height: usize,
    rgb: &[u8],
    image_size: usize,
    device: &candle_core::Device,
    dtype: candle_core::DType,
) -> ThgResult<candle_core::Tensor> {
    use candle_core::{DType, Tensor};
    use image::{DynamicImage, ImageBuffer, Rgb};

    let image = ImageBuffer::<Rgb<u8>, _>::from_raw(width as u32, height as u32, rgb.to_vec())
        .ok_or_else(|| {
            ThgError::new(
                "colpali_image_preprocess",
                "RGB bytes could not be interpreted as an image buffer",
            )
        })?;
    let image = DynamicImage::ImageRgb8(image)
        .resize_to_fill(
            image_size as u32,
            image_size as u32,
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8()
        .into_raw();
    Tensor::from_vec(
        image,
        (image_size, image_size, 3),
        &candle_core::Device::Cpu,
    )
    .and_then(|tensor| tensor.permute((2, 0, 1)))
    .and_then(|tensor| tensor.to_dtype(DType::F32))
    .and_then(|tensor| tensor.affine(2.0 / 255.0, -1.0))
    .and_then(|tensor| tensor.unsqueeze(0))
    .and_then(|tensor| tensor.to_device(device))
    .and_then(|tensor| tensor.to_dtype(dtype))
    .map_err(model_error)
}

#[cfg(feature = "colpali-candle")]
fn append_tensor_vectors(
    segment_id: &str,
    tensor: candle_core::Tensor,
    max_vectors: Option<usize>,
    out: &mut Vec<Vec<f32>>,
    error_code: &'static str,
) -> ThgResult<()> {
    let shape = tensor.dims().to_vec();
    let vectors = tensor
        .to_dtype(candle_core::DType::F32)
        .and_then(|tensor| tensor.to_vec3::<f32>())
        .map_err(model_error)?;
    if vectors.len() != 1 {
        return Err(ThgError::new(
            error_code,
            format!("segment {segment_id} produced shape {shape:?}, expected batch size 1"),
        ));
    }
    for vector in vectors.into_iter().next().unwrap_or_default() {
        push_if_within_budget(out, max_vectors, vector);
        if budget_exhausted(out.len(), max_vectors) {
            break;
        }
    }
    Ok(())
}

#[cfg(feature = "colpali-candle")]
fn model_error<E: std::fmt::Display>(error: E) -> ThgError {
    ThgError::new("colpali_model", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_producer_emits_bounded_text_vectors() {
        let mut producer = HashingMultiVectorProducer::new(16);
        let request = MultiVectorEncodeRequest {
            embedding_set_id: "mv:q".to_string(),
            content_id: "query".to_string(),
            segments: vec![MultiVectorInputSegment::Text {
                segment_id: "q".to_string(),
                text: "invoice total and payment amount".to_string(),
            }],
            max_vectors: Some(3),
        };

        let set = producer.encode(&request).expect("hash vectors");

        assert_eq!(set.embedding_set_id, "mv:q");
        assert_eq!(set.content_id, "query");
        assert_eq!(set.model_id, "colpali-hash-fixture");
        assert_eq!(set.vector_count(), 3);
        assert_eq!(set.dim().expect("dim"), 16);
        assert!(set.vectors.iter().flatten().all(|value| value.is_finite()));
    }

    #[test]
    fn hashing_producer_is_deterministic_for_image_segments() {
        let mut first = HashingMultiVectorProducer::new(8);
        let mut second = HashingMultiVectorProducer::new(8);
        let mut request = MultiVectorEncodeRequest::image_rgb8(
            "mv:page:1",
            "page:1",
            "image:page:1",
            2,
            2,
            vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        );
        request.max_vectors = Some(2);

        let first_set = first.encode(&request).expect("first image vectors");
        let second_set = second.encode(&request).expect("second image vectors");

        assert_eq!(first_set, second_set);
        assert_eq!(first_set.vector_count(), 1);
        assert_eq!(first_set.dim().expect("dim"), 8);
    }

    #[test]
    fn projection_bundle_keeps_exact_cold_and_binary_hot_refs() {
        let mut producer = HashingMultiVectorProducer::new(64);
        let request = MultiVectorEncodeRequest {
            embedding_set_id: "mv:page:1".to_string(),
            content_id: "page:1".to_string(),
            segments: vec![MultiVectorInputSegment::Text {
                segment_id: "body".to_string(),
                text: "graph nodes point to cold document bodies".to_string(),
            }],
            max_vectors: Some(4),
        };
        let set = producer.encode(&request).expect("vectors");

        let bundle = project_multivector_tiers(
            &set,
            Some("cold://exact/page-1.f32".to_string()),
            Some("hot://binary/page-1.bits".to_string()),
        )
        .expect("projection bundle");

        assert_eq!(
            bundle.manifest.exact_object_ref.as_deref(),
            Some("cold://exact/page-1.f32")
        );
        assert_eq!(
            bundle.manifest.binary_projection_ref.as_deref(),
            Some("hot://binary/page-1.bits")
        );
        assert_eq!(
            bundle.binary_projection.embedding_set_id,
            set.embedding_set_id
        );
        assert_eq!(bundle.binary_projection.vector_count, set.vector_count());
        assert!(bundle.manifest.exact_to_binary_byte_ratio() >= 32.0);
    }

    #[test]
    fn encode_request_rejects_malformed_rgb_payloads() {
        let mut producer = HashingMultiVectorProducer::default();
        let request =
            MultiVectorEncodeRequest::image_rgb8("mv:bad", "bad", "image:bad", 4, 4, vec![0; 7]);

        let err = producer.encode(&request).expect_err("invalid RGB length");

        assert_eq!(err.code, "multivector_encode_invalid_segment");
    }
}
