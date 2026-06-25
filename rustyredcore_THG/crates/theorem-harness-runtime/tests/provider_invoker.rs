use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use theorem_harness_core::{
    AgentHeadEndpoint, ContextMembranePrime, GroundedClaim, HeadCostProfile, HeadInvocationError,
    HeadInvocationKind, HeadInvocationRequest, HeadInvoker, HeadKind, HeadReliabilityProfile,
    HeadTransport, ResolvedAgentHead, TraceTier,
};
use theorem_harness_runtime::{CredentialResolver, EndpointMap, ProviderHeadInvoker};

#[derive(Debug)]
struct CapturedRequest {
    headers: String,
    body: Value,
}

struct MockChatCompletions {
    url: String,
    received: mpsc::Receiver<CapturedRequest>,
    handle: thread::JoinHandle<()>,
}

impl MockChatCompletions {
    fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                tx.send(read_request(&mut stream)).unwrap();
                write_response(&mut stream, response);
            }
        });
        Self {
            url,
            received: rx,
            handle,
        }
    }

    fn take_requests(self, count: usize) -> Vec<CapturedRequest> {
        let requests = (0..count)
            .map(|_| {
                self.received
                    .recv_timeout(Duration::from_secs(5))
                    .expect("mock provider request")
            })
            .collect();
        self.handle.join().unwrap();
        requests
    }
}

struct MockResponse {
    status: u16,
    body: String,
}

impl MockResponse {
    fn ok(content: &str) -> Self {
        Self {
            status: 200,
            body: json!({
                "model": "mock-model",
                "choices": [{
                    "message": { "content": content },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 7, "completion_tokens": 5, "total_tokens": 12 }
            })
            .to_string(),
        }
    }

    fn error(status: u16, detail: &str) -> Self {
        Self {
            status,
            body: detail.to_string(),
        }
    }
}

fn write_response(stream: &mut TcpStream, response: MockResponse) {
    let status_text = if response.status == 200 {
        "OK"
    } else {
        "ERROR"
    };
    let raw = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        status_text,
        response.body.len(),
        response.body
    );
    stream.write_all(raw.as_bytes()).unwrap();
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 1024];
    loop {
        let read = stream.read(&mut scratch).unwrap();
        assert!(read > 0, "request closed before body was complete");
        buffer.extend_from_slice(&scratch[..read]);
        if let Some((header_end, content_length)) = request_shape(&buffer) {
            let body_start = header_end + 4;
            if buffer.len() >= body_start + content_length {
                let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                let body = serde_json::from_slice(&buffer[body_start..body_start + content_length])
                    .unwrap();
                return CapturedRequest { headers, body };
            }
        }
    }
}

fn request_shape(buffer: &[u8]) -> Option<(usize, usize)> {
    let header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .or_else(|| {
            headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length:"))
        })?
        .trim()
        .parse()
        .ok()?;
    Some((header_end, content_length))
}

#[test]
fn openai_provider_request_uses_kind_system_message_and_parses_claims() {
    let secret = "provider-test-secret";
    let content = r#"First provider line.

Claims JSON:
[
  {"text":"the provider asserted this","provenance":"source:provider"}
]
"#;
    let mock = MockChatCompletions::start(vec![MockResponse::ok(content)]);
    let _env = ScopedEnv::new(vec![("TEST_PROVIDER_KEY", secret.to_string())]);
    let mut endpoints = EndpointMap::default();
    endpoints.insert("mistral", &HeadTransport::Api, mock.url.clone());
    let invoker = ProviderHeadInvoker::with_credentials_and_cost_units(
        endpoints,
        CredentialResolver::new(),
        1.0,
    )
    .unwrap();

    let receipt = invoker
        .invoke(request("mistral", HeadInvocationKind::Critique))
        .unwrap();

    assert_eq!(receipt.output_summary, "First provider line.");
    assert_eq!(
        receipt.claims,
        vec![GroundedClaim::new(
            "the provider asserted this",
            "source:provider"
        )]
    );
    assert_eq!(receipt.cost_units, 12.0);
    assert!(!serde_json::to_string(&receipt).unwrap().contains(secret));

    let requests = mock.take_requests(1);
    assert!(requests[0]
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer provider-test-secret"));
    assert_eq!(requests[0].body["model"], json!("mock-model"));
    assert_eq!(requests[0].body["messages"][0]["role"], json!("system"));
    assert!(requests[0].body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("one mind of Theorem"));
    assert_eq!(requests[0].body["messages"][1]["role"], json!("user"));
    assert!(requests[0].body["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Seed grounding claims"));
    assert!(requests[0].body["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Shared CRDT scratchpad"));
    assert!(requests[0].body["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Context membrane primes"));
}

#[test]
fn non_success_response_maps_to_provider_error_without_secret() {
    let secret = "provider-error-secret";
    let mock = MockChatCompletions::start(vec![MockResponse::error(429, "rate limited")]);
    let _env = ScopedEnv::new(vec![("TEST_PROVIDER_KEY", secret.to_string())]);
    let mut endpoints = EndpointMap::default();
    endpoints.insert("mistral", &HeadTransport::Api, mock.url.clone());
    let invoker = ProviderHeadInvoker::with_credentials_and_cost_units(
        endpoints,
        CredentialResolver::new(),
        1.0,
    )
    .unwrap();

    let error = invoker
        .invoke(request("mistral", HeadInvocationKind::Proposal))
        .unwrap_err();

    match error {
        HeadInvocationError::ProviderError { status, detail, .. } => {
            assert_eq!(status, 429);
            assert!(detail.contains("rate limited"));
            assert!(!detail.contains(secret));
        }
        other => panic!("expected provider error, got {other:?}"),
    }
    let requests = mock.take_requests(1);
    assert!(!requests[0].body.to_string().contains(secret));
}

#[test]
fn empty_env_credential_is_a_provider_error() {
    let _env = ScopedEnv::new(vec![("TEST_PROVIDER_KEY", " ".to_string())]);
    let mut endpoints = EndpointMap::default();
    endpoints.insert(
        "mistral",
        &HeadTransport::Api,
        "http://127.0.0.1:9/v1/chat/completions",
    );
    let invoker = ProviderHeadInvoker::with_credentials_and_cost_units(
        endpoints,
        CredentialResolver::new(),
        1.0,
    )
    .unwrap();

    let error = invoker
        .invoke(request("mistral", HeadInvocationKind::Proposal))
        .unwrap_err();

    match error {
        HeadInvocationError::ProviderError { status, detail, .. } => {
            assert_eq!(status, 0);
            assert!(detail.contains("TEST_PROVIDER_KEY"));
        }
        other => panic!("expected provider error, got {other:?}"),
    }
}

#[test]
fn local_transport_uses_openai_compatible_endpoint_without_bearer() {
    let content = r#"local response

Claims JSON:
[{"text":"local claim","provenance":"local:gemma"}]
"#;
    let mock = MockChatCompletions::start(vec![MockResponse::ok(content)]);
    let mut endpoints = EndpointMap::default();
    endpoints.insert("gemma", &HeadTransport::Local, mock.url.clone());
    let invoker = ProviderHeadInvoker::with_credentials_and_cost_units(
        endpoints,
        CredentialResolver::new(),
        1.0,
    )
    .unwrap();

    let receipt = invoker
        .invoke(request_with_transport(
            "gemma",
            HeadTransport::Local,
            "gemma3:latest",
            "",
            HeadInvocationKind::Proposal,
        ))
        .unwrap();

    assert_eq!(receipt.payload["transport"], json!("local"));
    assert_eq!(receipt.claims[0].provenance, "local:gemma");
    let requests = mock.take_requests(1);
    assert!(!requests[0]
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer"));
}

#[test]
fn hosted_transport_uses_configured_endpoint_and_bearer() {
    let secret = "hosted-provider-secret";
    let mock = MockChatCompletions::start(vec![MockResponse::ok("hosted response")]);
    let _env = ScopedEnv::new(vec![("TEST_PROVIDER_KEY", secret.to_string())]);
    let mut endpoints = EndpointMap::default();
    endpoints.insert("runpod", &HeadTransport::Hosted, mock.url.clone());
    let invoker = ProviderHeadInvoker::with_credentials_and_cost_units(
        endpoints,
        CredentialResolver::new(),
        1.0,
    )
    .unwrap();

    let receipt = invoker
        .invoke(request_with_transport(
            "runpod",
            HeadTransport::Hosted,
            "open-weight-model",
            "env:TEST_PROVIDER_KEY",
            HeadInvocationKind::Proposal,
        ))
        .unwrap();

    assert_eq!(receipt.payload["transport"], json!("hosted"));
    let requests = mock.take_requests(1);
    assert!(requests[0]
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer hosted-provider-secret"));
}

fn request(provider: &str, kind: HeadInvocationKind) -> HeadInvocationRequest {
    request_with_transport(
        provider,
        HeadTransport::Api,
        "mock-model",
        "env:TEST_PROVIDER_KEY",
        kind,
    )
}

fn request_with_transport(
    provider: &str,
    transport: HeadTransport,
    model: &str,
    credential_ref: &str,
    kind: HeadInvocationKind,
) -> HeadInvocationRequest {
    HeadInvocationRequest::new_with_context(
        head(provider, transport, model, credential_ref),
        kind,
        "answer this task",
        2,
        vec!["scratchrev:1".to_string()],
        vec![theorem_harness_core::RevisionContext {
            revision_id: "scratchrev:1".to_string(),
            kind: HeadInvocationKind::Proposal,
            output_summary: "prior proposal".to_string(),
            payload: {
                let mut payload = serde_json::Map::new();
                payload.insert("text".to_string(), Value::String("prior body".to_string()));
                payload
            },
        }],
        vec![GroundedClaim::new("seed claim", "seed:claim")],
        "2026-06-19T00:00:00Z",
    )
    .with_context_membrane(vec![ContextMembranePrime::new(
        "context:test",
        "provider test",
        "provider prompt should receive run-start context",
        "test:provider_invoker",
        1.0,
    )])
}

fn head(
    provider: &str,
    transport: HeadTransport,
    model: &str,
    credential_ref: &str,
) -> ResolvedAgentHead {
    ResolvedAgentHead {
        head_id: provider.to_string(),
        display_name: provider.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        kind: HeadKind::ReasoningCore,
        endpoint: AgentHeadEndpoint {
            transport,
            target: "fake://target".to_string(),
            fake: true,
        },
        credential_ref: credential_ref.to_string(),
        capabilities: Vec::new(),
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: Vec::new(),
        trace_tier: TraceTier::Receipt,
    }
}

struct ScopedEnv {
    saved: Vec<(String, Option<String>)>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl ScopedEnv {
    fn new(pairs: Vec<(&'static str, String)>) -> Self {
        let guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = pairs
            .iter()
            .map(|(name, _)| {
                let value = std::env::var(name).ok();
                std::env::remove_var(name);
                ((*name).to_string(), value)
            })
            .collect::<Vec<_>>();
        for (name, value) in pairs {
            std::env::set_var(name, value);
        }
        Self {
            saved,
            _guard: guard,
        }
    }
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
