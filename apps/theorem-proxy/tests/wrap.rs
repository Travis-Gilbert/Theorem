//! Acceptance for `theorem-proxy wrap` (Phase C.1 one-command connect): the proxy comes
//! up and the wrapped child runs with ANTHROPIC_BASE_URL pointed at it -- no manual
//! export. Proven by wrapping a shell command that records the env var it saw.

use std::net::{SocketAddr, TcpListener};

use theorem_proxy::{run_wrapped, ProxyConfig};

#[tokio::test]
async fn wrap_sets_anthropic_base_url_and_runs_the_command() {
    // Reserve a free port, then release it so the proxy can bind it.
    let addr: SocketAddr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let tmp = std::env::temp_dir().join(format!("theorem-proxy-wrap-{}.txt", addr.port()));
    let _ = std::fs::remove_file(&tmp);

    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf '%s' \"$ANTHROPIC_BASE_URL\" > \"$1\"".to_string(),
        "write-url".to_string(),
        tmp.to_string_lossy().to_string(),
    ];
    let code = run_wrapped(addr, ProxyConfig::default(), command)
        .await
        .unwrap();

    assert_eq!(code, 0, "wrapped command exited cleanly");
    let seen = std::fs::read_to_string(&tmp).unwrap();
    assert_eq!(
        seen,
        format!("http://{addr}"),
        "the child saw ANTHROPIC_BASE_URL = the proxy"
    );
    let _ = std::fs::remove_file(&tmp);
}
