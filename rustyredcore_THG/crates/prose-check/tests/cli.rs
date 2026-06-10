use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_emits_style_receipt_json_from_stdin() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_prose-check"))
        .args(["--register", "plain", "--identifiers", "prose-check"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn prose-check");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"In order to test prose-check, keep prose-check exact.")
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");

    assert!(output.status.success());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).expect("receipt json");
    assert_eq!(receipt["register"], "Plain");
    assert_eq!(receipt["fidelity"]["preserved"], true);
    assert!(!receipt["clutter_hits"].as_array().unwrap().is_empty());
    assert!(receipt["pack_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn cli_emits_pack_payload_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_prose-check"))
        .arg("--pack-payload")
        .output()
        .expect("run prose-check");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("pack payload json");
    assert_eq!(payload["id"], "skill-pack:writing-engineering-prose-v0.1");
    assert_eq!(payload["kind"], "skill_pack");
    assert!(payload["metadata"]["pack_content_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}
