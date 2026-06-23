use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn cli_emits_pack_payload_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_design-check"))
        .arg("--pack-payload")
        .output()
        .expect("run design-check");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("pack payload json");
    assert_eq!(payload["id"], "skill-pack:design-engineering-general-v0.1");
    assert_eq!(payload["kind"], "skill_pack");
    assert!(payload["metadata"]["pack_content_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(payload["metadata"]["marketplace_export"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file["path"] == "theorems-harness/skills/design-engineering/provenance.json"));
}

#[test]
fn cli_css_static_reports_bad_contrast() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_design-check"))
        .arg("--css-static")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn design-check");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b".bad { color: #777; background: #777; }")
        .expect("write CSS");
    let output = child.wait_with_output().expect("wait");

    assert!(output.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("css_static report json");
    assert_eq!(report["checker"], "css_static");
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| {
            finding["rule_id"] == "contrast_minimum_met" && finding["status"] == "failed"
        }));
}
