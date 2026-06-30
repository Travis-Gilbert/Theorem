use std::env;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use rustyred_thg_core::{
    GraphStoreError, GraphStoreResult, RedCorePayloadBackend, REDCORE_LOCAL_PAYLOAD_BACKEND,
    REDCORE_RAILWAY_BUCKETS_PAYLOAD_BACKEND,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub struct RailwayBucketPayloadBackend {
    endpoint: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
    region: String,
    client: Client,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RailwayBucketPayloadSettings {
    pub endpoint: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
}

impl RailwayBucketPayloadBackend {
    pub fn new(settings: RailwayBucketPayloadSettings) -> GraphStoreResult<Self> {
        let endpoint = settings.endpoint.trim_end_matches('/').to_string();
        if endpoint.is_empty() {
            return Err(GraphStoreError::new(
                "payload_bucket_config",
                "payload bucket endpoint is empty",
            ));
        }
        if settings.bucket.trim().is_empty() {
            return Err(GraphStoreError::new(
                "payload_bucket_config",
                "payload bucket name is empty",
            ));
        }
        Ok(Self {
            endpoint,
            bucket: settings.bucket,
            access_key_id: settings.access_key_id,
            secret_access_key: settings.secret_access_key,
            region: settings.region,
            client: Client::new(),
        })
    }

    fn object_key(content_hash: &str) -> String {
        format!("payloads/{}.bin", safe_key_segment(content_hash))
    }

    fn object_url(&self, content_hash: &str) -> String {
        format!(
            "{}/{}/{}",
            self.endpoint,
            uri_encode_segment(&self.bucket),
            Self::object_key(content_hash)
        )
    }

    fn signed_headers(
        &self,
        method: &str,
        url: &reqwest::Url,
        body: &[u8],
        now: OffsetDateTime,
    ) -> GraphStoreResult<SignedHeaders> {
        let amz_date = format_amz_date(now)?;
        let date_scope = format_date_scope(now)?;
        let payload_sha = sha256_hex(body);
        let host = canonical_host(url)?;
        let canonical_uri = url.path();
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_sha}\nx-amz-date:{amz_date}\n");
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_sha}"
        );
        let credential_scope = format!("{date_scope}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let signing_key = signing_key(&self.secret_access_key, &date_scope, &self.region)?;
        let signature = hmac_sha256_hex(&signing_key, string_to_sign.as_bytes())?;
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id
        );
        Ok(SignedHeaders {
            authorization,
            amz_date,
            payload_sha,
        })
    }
}

impl RedCorePayloadBackend for RailwayBucketPayloadBackend {
    fn backend_name(&self) -> &'static str {
        REDCORE_RAILWAY_BUCKETS_PAYLOAD_BACKEND
    }

    fn put_payload_bytes(&self, content_hash: &str, body: &[u8]) -> GraphStoreResult<()> {
        let raw_url = self.object_url(content_hash);
        let url = reqwest::Url::parse(&raw_url).map_err(|error| {
            GraphStoreError::new("payload_bucket_url", format!("invalid bucket URL: {error}"))
        })?;
        let signed = self.signed_headers("PUT", &url, body, OffsetDateTime::now_utc())?;
        let response = self
            .client
            .put(url)
            .header("Authorization", signed.authorization)
            .header("x-amz-date", signed.amz_date)
            .header("x-amz-content-sha256", signed.payload_sha)
            .body(body.to_vec())
            .send()
            .map_err(|error| GraphStoreError::new("payload_bucket_put", error.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(GraphStoreError::new(
                "payload_bucket_put",
                format!("bucket PUT failed with HTTP {}", response.status()),
            ))
        }
    }

    fn get_payload_bytes(&self, content_hash: &str) -> GraphStoreResult<Option<Vec<u8>>> {
        let raw_url = self.object_url(content_hash);
        let url = reqwest::Url::parse(&raw_url).map_err(|error| {
            GraphStoreError::new("payload_bucket_url", format!("invalid bucket URL: {error}"))
        })?;
        let signed = self.signed_headers("GET", &url, &[], OffsetDateTime::now_utc())?;
        let response = self
            .client
            .get(url)
            .header("Authorization", signed.authorization)
            .header("x-amz-date", signed.amz_date)
            .header("x-amz-content-sha256", signed.payload_sha)
            .send()
            .map_err(|error| GraphStoreError::new("payload_bucket_get", error.to_string()))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(GraphStoreError::new(
                "payload_bucket_get",
                format!("bucket GET failed with HTTP {}", response.status()),
            ));
        }
        response
            .bytes()
            .map(|bytes| Some(bytes.to_vec()))
            .map_err(|error| GraphStoreError::new("payload_bucket_get", error.to_string()))
    }
}

#[derive(Clone, Debug)]
struct SignedHeaders {
    authorization: String,
    amz_date: String,
    payload_sha: String,
}

pub fn payload_backend_from_env() -> GraphStoreResult<Option<Arc<dyn RedCorePayloadBackend>>> {
    let mode = env_first_optional(&["RUSTY_RED_PAYLOAD_BACKEND", "RUSTYRED_THG_PAYLOAD_BACKEND"]);
    let mode_normalized = mode.as_deref().map(normalize_mode);
    let disabled = matches!(
        mode_normalized.as_deref(),
        Some("none" | "disabled" | REDCORE_LOCAL_PAYLOAD_BACKEND)
    );
    if disabled {
        return Ok(None);
    }

    let settings = RailwayBucketPayloadSettings::from_env();
    let explicit_bucket = matches!(
        mode_normalized.as_deref(),
        Some("railway_buckets" | "railway" | "s3" | "bucket")
    );
    if mode_normalized.is_some() && !explicit_bucket {
        return Err(GraphStoreError::new(
            "payload_backend_config",
            format!(
                "unsupported payload backend {}; expected railway_buckets, s3, local_data_dir, or none",
                mode.unwrap_or_default()
            ),
        ));
    }
    match (explicit_bucket, settings) {
        (_, Ok(Some(settings))) => Ok(Some(Arc::new(RailwayBucketPayloadBackend::new(settings)?))),
        (true, Ok(None)) => Err(GraphStoreError::new(
            "payload_bucket_config",
            "RUSTY_RED_PAYLOAD_BACKEND=railway_buckets requires bucket endpoint, name, access key, and secret key",
        )),
        (true, Err(error)) => Err(error),
        (false, Ok(None)) => Ok(None),
        (false, Err(_)) => Ok(None),
    }
}

impl RailwayBucketPayloadSettings {
    pub fn from_env() -> GraphStoreResult<Option<Self>> {
        let endpoint = env_first_optional(&[
            "RUSTY_RED_PAYLOAD_BUCKET_ENDPOINT",
            "RUSTYRED_THG_PAYLOAD_BUCKET_ENDPOINT",
            "BUCKET_ENDPOINT",
            "GLOBAL_S3_ENDPOINT",
            "AWS_ENDPOINT_URL_S3",
        ]);
        let bucket = env_first_optional(&[
            "RUSTY_RED_PAYLOAD_BUCKET_NAME",
            "RUSTYRED_THG_PAYLOAD_BUCKET_NAME",
            "BUCKET_NAME",
            "GLOBAL_S3_BUCKET",
        ]);
        let access_key_id = env_first_optional(&[
            "RUSTY_RED_PAYLOAD_BUCKET_ACCESS_KEY_ID",
            "RUSTYRED_THG_PAYLOAD_BUCKET_ACCESS_KEY_ID",
            "BUCKET_ACCESS_KEY_ID",
            "AWS_ACCESS_KEY_ID",
        ]);
        let secret_access_key = env_first_optional(&[
            "RUSTY_RED_PAYLOAD_BUCKET_SECRET_ACCESS_KEY",
            "RUSTYRED_THG_PAYLOAD_BUCKET_SECRET_ACCESS_KEY",
            "BUCKET_SECRET_ACCESS_KEY",
            "AWS_SECRET_ACCESS_KEY",
        ]);
        let present = [
            endpoint.as_ref(),
            bucket.as_ref(),
            access_key_id.as_ref(),
            secret_access_key.as_ref(),
        ]
        .into_iter()
        .filter(|value| value.is_some())
        .count();
        if present == 0 {
            return Ok(None);
        }
        if present != 4 {
            return Err(GraphStoreError::new(
                "payload_bucket_config",
                "payload bucket configuration is partial; endpoint, bucket, access key, and secret key are all required",
            ));
        }
        Ok(Some(Self {
            endpoint: endpoint.unwrap(),
            bucket: bucket.unwrap(),
            access_key_id: access_key_id.unwrap(),
            secret_access_key: secret_access_key.unwrap(),
            region: env_first_optional(&[
                "RUSTY_RED_PAYLOAD_BUCKET_REGION",
                "RUSTYRED_THG_PAYLOAD_BUCKET_REGION",
                "BUCKET_REGION",
                "AWS_REGION",
                "AWS_DEFAULT_REGION",
                "REGION",
            ])
            .unwrap_or_else(|| "auto".to_string()),
        }))
    }
}

fn normalize_mode(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn env_first_optional(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn canonical_host(url: &reqwest::Url) -> GraphStoreResult<String> {
    let host = url.host_str().ok_or_else(|| {
        GraphStoreError::new("payload_bucket_url", "bucket URL has no host".to_string())
    })?;
    let include_port = match (url.scheme(), url.port()) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => false,
        (_, Some(_)) => true,
    };
    Ok(if include_port {
        format!("{host}:{}", url.port().unwrap())
    } else {
        host.to_string()
    })
}

fn format_amz_date(now: OffsetDateTime) -> GraphStoreResult<String> {
    Ok(format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    ))
}

fn format_date_scope(now: OffsetDateTime) -> GraphStoreResult<String> {
    Ok(format!(
        "{:04}{:02}{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    ))
}

fn signing_key(secret: &str, date_scope: &str, region: &str) -> GraphStoreResult<Vec<u8>> {
    let date_key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_scope.as_bytes())?;
    let region_key = hmac_sha256(&date_key, region.as_bytes())?;
    let service_key = hmac_sha256(&region_key, b"s3")?;
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> GraphStoreResult<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| GraphStoreError::new("payload_bucket_sign", error.to_string()))?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> GraphStoreResult<String> {
    hmac_sha256(key, message).map(hex::encode)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn safe_key_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn uri_encode_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_nibble(byte >> 4));
            encoded.push(hex_nibble(byte & 0x0f));
        }
    }
    encoded
}

fn hex_nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + (value - 10)) as char,
        _ => unreachable!("hex nibble is always <= 15"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    use rustyred_thg_core::{NodeRecord, RedCoreGraphStore, RedCorePayloadPointer};
    use serde_json::json;

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        body: Vec<u8>,
        authorization_present: bool,
        amz_date_present: bool,
        payload_sha: Option<String>,
    }

    #[test]
    fn railway_bucket_object_key_is_content_hash_derived() {
        assert_eq!(
            RailwayBucketPayloadBackend::object_key("sha256:abc123"),
            "payloads/sha256_abc123.bin"
        );
    }

    #[test]
    fn uri_encode_segment_keeps_s3_safe_chars() {
        assert_eq!(uri_encode_segment("bucket.name"), "bucket.name");
        assert_eq!(uri_encode_segment("needs space"), "needs%20space");
    }

    #[test]
    fn railway_bucket_backend_round_trips_graph_payload_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut stored_body = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                let response_body = if request.method == "PUT" {
                    stored_body = request.body.clone();
                    Vec::new()
                } else {
                    stored_body.clone()
                };
                write_http_response(&mut stream, &response_body);
                request_tx.send(request).unwrap();
            }
        });

        let backend = Arc::new(
            RailwayBucketPayloadBackend::new(RailwayBucketPayloadSettings {
                endpoint,
                bucket: "test-bucket".to_string(),
                access_key_id: "test-access-key".to_string(),
                secret_access_key: "test-secret-key".to_string(),
                region: "auto".to_string(),
            })
            .unwrap(),
        );
        let payload = b"railway bucket payload bytes";
        let mut store = RedCoreGraphStore::memory();
        store.set_payload_backend(backend);
        store
            .upsert_node_with_payload_bytes(
                NodeRecord::new(
                    "node:bucket-payload",
                    ["Payload"],
                    json!({ "title": "bucket" }),
                ),
                "payload",
                payload,
            )
            .unwrap();

        let node = store.get_node("node:bucket-payload").unwrap().unwrap();
        let pointer = RedCorePayloadPointer::from_value(&node.properties["payload"]).unwrap();
        assert_eq!(pointer.backend, REDCORE_RAILWAY_BUCKETS_PAYLOAD_BACKEND);
        assert_eq!(pointer.byte_len, payload.len());
        assert!(!node.properties.to_string().contains("payload bytes"));
        assert_eq!(
            store
                .read_node_payload_bytes("node:bucket-payload", "payload")
                .unwrap()
                .unwrap(),
            payload
        );

        let put = request_rx.recv().unwrap();
        let get = request_rx.recv().unwrap();
        let expected_path = format!(
            "/test-bucket/{}",
            RailwayBucketPayloadBackend::object_key(&pointer.content_hash)
        );
        assert_eq!(put.method, "PUT");
        assert_eq!(put.path, expected_path);
        assert_eq!(put.body, payload);
        assert!(put.authorization_present);
        assert!(put.amz_date_present);
        let expected_put_sha = sha256_hex(payload);
        assert_eq!(put.payload_sha.as_deref(), Some(expected_put_sha.as_str()));
        assert_eq!(get.method, "GET");
        assert_eq!(get.path, expected_path);
        assert!(get.authorization_present);
        assert!(get.amz_date_present);
        let expected_get_sha = sha256_hex(&[]);
        assert_eq!(get.payload_sha.as_deref(), Some(expected_get_sha.as_str()));

        server.join().unwrap();
    }

    fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
        let mut raw = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 512];
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "HTTP client closed before sending headers");
            raw.extend_from_slice(&chunk[..read]);
            if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers_raw = String::from_utf8(raw[..header_end].to_vec()).unwrap();
        let mut lines = headers_raw.split("\r\n");
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let mut content_length = 0_usize;
        let mut authorization_present = false;
        let mut amz_date_present = false;
        let mut payload_sha = None;
        for line in lines.filter(|line| !line.is_empty()) {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = value.trim().parse().unwrap(),
                "authorization" => authorization_present = true,
                "x-amz-date" => amz_date_present = true,
                "x-amz-content-sha256" => payload_sha = Some(value.trim().to_string()),
                _ => {}
            }
        }
        let mut body = raw[header_end..].to_vec();
        while body.len() < content_length {
            let mut chunk = vec![0_u8; content_length - body.len()];
            stream.read_exact(&mut chunk).unwrap();
            body.extend_from_slice(&chunk);
        }
        body.truncate(content_length);
        CapturedRequest {
            method,
            path,
            body,
            authorization_present,
            amz_date_present,
            payload_sha,
        }
    }

    fn write_http_response(stream: &mut TcpStream, body: &[u8]) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    }
}
