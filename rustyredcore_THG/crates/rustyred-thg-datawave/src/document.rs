use std::fmt;
use std::path::PathBuf;

use liteparse::config::{ImageMode, LiteParseConfig};
use liteparse::types::PdfInput;
use liteparse::{LiteParse, OutputFormat};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    DatawaveError, DatawaveIngest, FieldConfig, FieldType, IndexPolicy, IngestOutcome, IngestStats,
    JsonHelper, MaterializeConfig, RawRecord,
};

pub const DOCUMENT_DATA_TYPE: &str = "document";
pub const DOCUMENT_PARSE_VERSION: &str = "liteparse:a8288d09";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentParseState {
    Parsed,
    NeedsHeavyPath,
}

impl DocumentParseState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Parsed => "parsed",
            Self::NeedsHeavyPath => "needs_heavy_path",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentComplexityPage {
    pub page_number: usize,
    pub needs_ocr: bool,
    pub reasons: Vec<String>,
    pub text_length: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentParseRecord {
    pub document_id: String,
    pub source_uri: String,
    pub content_type: String,
    pub parser: String,
    pub parse_state: DocumentParseState,
    pub confidence: String,
    pub markdown: String,
    pub page_count: usize,
    pub hard_reasons: Vec<String>,
    pub complexity: Vec<DocumentComplexityPage>,
}

impl DocumentParseRecord {
    pub fn to_raw_record(&self, event_time_ms: i64) -> RawRecord {
        RawRecord::json(DOCUMENT_DATA_TYPE, json!(self), event_time_ms)
            .with_external_id(self.document_id.clone())
    }
}

#[derive(Clone, Debug)]
pub enum DocumentParseInput {
    PdfBytes {
        document_id: String,
        source_uri: String,
        content_type: String,
        bytes: Vec<u8>,
    },
    Path {
        document_id: String,
        path: PathBuf,
        content_type: String,
    },
}

#[derive(Debug)]
pub enum DocumentParseError {
    Liteparse(String),
}

impl fmt::Display for DocumentParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Liteparse(message) => write!(f, "liteparse failed: {message}"),
        }
    }
}

impl std::error::Error for DocumentParseError {}

pub fn document_field_config() -> FieldConfig {
    FieldConfig::new()
        .with_default_type(FieldType::LcText)
        .with_default_policy(IndexPolicy::INDEXED)
        .with_field("document_id", FieldType::NoOp, IndexPolicy::INDEXED)
        .with_field(
            "source_uri",
            FieldType::Text,
            IndexPolicy::INDEXED.with_reverse(),
        )
        .with_field("content_type", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("parser", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("parse_state", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field("confidence", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field(
            "markdown",
            FieldType::LcText,
            IndexPolicy::INDEXED.with_tokenized(),
        )
        .with_field("page_count", FieldType::Number, IndexPolicy::INDEXED)
        .with_field("hard_reasons", FieldType::LcText, IndexPolicy::INDEXED)
        .with_field(
            "complexity.reasons",
            FieldType::LcText,
            IndexPolicy::INDEXED,
        )
}

pub fn document_datawave_ingest(tenant_id: Option<String>) -> DatawaveIngest {
    let mut config = MaterializeConfig::new(tenant_id).with_generation(0);
    config.source = "rustyred-thg-datawave:document".to_string();
    let mut ingest = DatawaveIngest::new(config);
    ingest.register(Box::new(JsonHelper::new(
        DOCUMENT_DATA_TYPE,
        document_field_config(),
    )));
    ingest
}

pub fn ingest_document_record<S: GraphStore>(
    store: &mut S,
    record: &DocumentParseRecord,
    tenant_id: Option<String>,
) -> Result<IngestOutcome, DatawaveError> {
    let ingest = document_datawave_ingest(tenant_id);
    let mut stats = IngestStats::new();
    ingest.ingest_record(store, &record.to_raw_record(0), &mut stats)
}

pub async fn parse_document_with_liteparse(
    input: DocumentParseInput,
) -> Result<DocumentParseRecord, DocumentParseError> {
    let (document_id, source_uri, content_type, pdf_input) = match input {
        DocumentParseInput::PdfBytes {
            document_id,
            source_uri,
            content_type,
            bytes,
        } => (
            document_id,
            source_uri,
            content_type,
            PdfInput::Bytes(bytes),
        ),
        DocumentParseInput::Path {
            document_id,
            path,
            content_type,
        } => {
            let source_uri = path.to_string_lossy().into_owned();
            (
                document_id,
                source_uri,
                content_type,
                PdfInput::Path(path.to_string_lossy().into_owned()),
            )
        }
    };

    let parser = LiteParse::new(LiteParseConfig {
        output_format: OutputFormat::Markdown,
        ocr_enabled: false,
        quiet: true,
        image_mode: ImageMode::Placeholder,
        ..Default::default()
    });

    let complexity = parser
        .is_complex(clone_pdf_input(&pdf_input))
        .await
        .map_err(|err| DocumentParseError::Liteparse(err.to_string()))?;
    let pages = complexity
        .iter()
        .map(complexity_page_from_liteparse)
        .collect::<Vec<_>>();
    let hard_reasons = pages
        .iter()
        .flat_map(|page| page.reasons.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    if pages.iter().any(|page| page.needs_ocr) {
        return Ok(DocumentParseRecord {
            document_id,
            source_uri,
            content_type,
            parser: DOCUMENT_PARSE_VERSION.to_string(),
            parse_state: DocumentParseState::NeedsHeavyPath,
            confidence: "low".to_string(),
            markdown: String::new(),
            page_count: pages.len(),
            hard_reasons,
            complexity: pages,
        });
    }

    let parsed = parser
        .parse_input(pdf_input)
        .await
        .map_err(|err| DocumentParseError::Liteparse(err.to_string()))?;
    Ok(DocumentParseRecord {
        document_id,
        source_uri,
        content_type,
        parser: DOCUMENT_PARSE_VERSION.to_string(),
        parse_state: DocumentParseState::Parsed,
        confidence: "high".to_string(),
        markdown: parsed.text,
        page_count: parsed.pages.len(),
        hard_reasons,
        complexity: pages,
    })
}

pub fn document_record_value(record: &RawRecord) -> Option<&Value> {
    match &record.body {
        crate::RecordBody::Json(value) if record.data_type == DOCUMENT_DATA_TYPE => Some(value),
        _ => None,
    }
}

fn clone_pdf_input(input: &PdfInput) -> PdfInput {
    match input {
        PdfInput::Path(path) => PdfInput::Path(path.clone()),
        PdfInput::Bytes(bytes) => PdfInput::Bytes(bytes.clone()),
    }
}

fn complexity_page_from_liteparse(
    page: &liteparse::ocr_merge::PageComplexityStats,
) -> DocumentComplexityPage {
    DocumentComplexityPage {
        page_number: page.page_number,
        needs_ocr: page.needs_ocr,
        reasons: page
            .reasons
            .iter()
            .map(|reason| reason.as_str().to_string())
            .collect(),
        text_length: page.text_length,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IngestStats;
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn document_records_land_as_datawave_document_facts() {
        let record = DocumentParseRecord {
            document_id: "doc:1".to_string(),
            source_uri: "file:///tmp/doc.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            parser: DOCUMENT_PARSE_VERSION.to_string(),
            parse_state: DocumentParseState::Parsed,
            confidence: "high".to_string(),
            markdown: "# Title\n\nSearchable body".to_string(),
            page_count: 1,
            hard_reasons: Vec::new(),
            complexity: vec![DocumentComplexityPage {
                page_number: 1,
                needs_ocr: false,
                reasons: Vec::new(),
                text_length: 21,
            }],
        };
        let ingest = document_datawave_ingest(Some("tenant".to_string()));
        let mut store = InMemoryGraphStore::default();
        let mut stats = IngestStats::new();
        let outcome = ingest
            .ingest_record(&mut store, &record.to_raw_record(0), &mut stats)
            .unwrap();
        assert!(outcome.fields_written >= 8);
        assert!(stats.field_names().contains(&"markdown"));
    }

    #[tokio::test]
    async fn liteparse_parses_text_pdf_to_markdown_record() {
        let parsed = parse_document_with_liteparse(DocumentParseInput::PdfBytes {
            document_id: "text".to_string(),
            source_uri: "memory://text.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            bytes: text_pdf(&"Harness search parse primitive ".repeat(120)),
        })
        .await
        .unwrap();
        assert_eq!(parsed.parse_state, DocumentParseState::Parsed);
        assert_eq!(parsed.confidence, "high");
        assert!(parsed.markdown.contains("Harness search parse primitive"));
        assert!(!parsed.complexity.iter().any(|page| page.needs_ocr));
    }

    #[tokio::test]
    async fn liteparse_flags_blank_pdf_as_hard_document() {
        let parsed = parse_document_with_liteparse(DocumentParseInput::PdfBytes {
            document_id: "blank".to_string(),
            source_uri: "memory://blank.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            bytes: blank_pdf(),
        })
        .await
        .unwrap();
        assert_eq!(parsed.parse_state, DocumentParseState::NeedsHeavyPath);
        assert!(parsed.hard_reasons.iter().any(|reason| reason == "no-text"));
        assert!(parsed.markdown.is_empty());
    }

    fn blank_pdf() -> Vec<u8> {
        br#"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >> endobj
trailer << /Root 1 0 R >>
%%EOF
"#
        .to_vec()
    }

    fn text_pdf(text: &str) -> Vec<u8> {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let stream = format!("BT /F1 18 Tf 50 120 Td ({escaped}) Tj ET");
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        push_obj(
            &mut pdf,
            &mut offsets,
            1,
            "<< /Type /Catalog /Pages 2 0 R >>",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            2,
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            3,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            4,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            5,
            &format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ),
        );
        let xref_offset = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer << /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n")
                .as_bytes(),
        );
        pdf
    }

    fn push_obj(pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, id: usize, body: &str) {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
    }
}
