use design_check::{
    css_static_report, design_audit, design_drift_from_json, design_engineering_pack_payload,
    design_fact_set_from_json, design_html_report, design_tokens_dtcg, design_tokens_tailwind,
    fixture_reports, lower_css, lower_tokens_json, token_lint_report, CssStaticInput, PACK_ID,
};
use std::io::{self, Read};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut mode = Mode::CssStatic;
    let mut parent_hash: Option<String> = None;
    let mut token_json: Option<String> = None;
    let mut baseline_json: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pack-payload" => mode = Mode::PackPayload,
            "--fixture-report" => mode = Mode::FixtureReport,
            "--css-static" => mode = Mode::CssStatic,
            "--token-lint" => mode = Mode::TokenLint,
            "--lower-css" => mode = Mode::LowerCss,
            "--lower-tokens" => mode = Mode::LowerTokens,
            "--audit-facts" => mode = Mode::AuditFacts,
            "--drift-facts" => mode = Mode::DriftFacts,
            "--tokens-dtcg" => mode = Mode::TokensDtcg,
            "--tokens-tailwind" => mode = Mode::TokensTailwind,
            "--html-report" => mode = Mode::HtmlReport,
            "--parent-hash" => {
                parent_hash = Some(
                    args.next()
                        .ok_or_else(|| "--parent-hash requires a content hash".to_string())?,
                );
            }
            "--tokens-json" => {
                token_json = Some(
                    args.next()
                        .ok_or_else(|| "--tokens-json requires JSON text".to_string())?,
                );
            }
            "--baseline-json" => {
                baseline_json = Some(
                    args.next()
                        .ok_or_else(|| "--baseline-json requires JSON text".to_string())?,
                );
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let output = match mode {
        Mode::PackPayload => design_engineering_pack_payload(parent_hash.as_deref()),
        Mode::FixtureReport => serde_json::to_value(fixture_reports())
            .map_err(|error| format!("failed to serialize fixture report: {error}"))?,
        Mode::CssStatic => {
            let css = read_stdin()?;
            serde_json::to_value(css_static_report(CssStaticInput {
                css,
                token_json,
                ..CssStaticInput::default()
            }))
            .map_err(|error| format!("failed to serialize css_static report: {error}"))?
        }
        Mode::TokenLint => {
            let css = read_stdin()?;
            serde_json::to_value(token_lint_report(CssStaticInput {
                css,
                token_json,
                ..CssStaticInput::default()
            }))
            .map_err(|error| format!("failed to serialize token_lint report: {error}"))?
        }
        Mode::LowerCss => {
            let css = read_stdin()?;
            serde_json::to_value(lower_css("stdin://css", &css))
                .map_err(|error| format!("failed to serialize CSS atoms: {error}"))?
        }
        Mode::LowerTokens => {
            let tokens = read_stdin()?;
            serde_json::to_value(lower_tokens_json("stdin://tokens", &tokens)?)
                .map_err(|error| format!("failed to serialize token atoms: {error}"))?
        }
        Mode::AuditFacts => {
            let input = read_stdin()?;
            let facts = design_fact_set_from_json(&input)?;
            serde_json::to_value(design_audit(&facts))
                .map_err(|error| format!("failed to serialize audit report: {error}"))?
        }
        Mode::DriftFacts => {
            let candidate = read_stdin()?;
            let baseline = baseline_json
                .as_deref()
                .ok_or_else(|| "--drift-facts requires --baseline-json JSON".to_string())?;
            serde_json::to_value(design_drift_from_json(baseline, &candidate)?)
                .map_err(|error| format!("failed to serialize drift report: {error}"))?
        }
        Mode::TokensDtcg => {
            let input = read_stdin()?;
            let facts = design_fact_set_from_json(&input)?;
            design_tokens_dtcg(&facts)
        }
        Mode::TokensTailwind => {
            let input = read_stdin()?;
            let facts = design_fact_set_from_json(&input)?;
            design_tokens_tailwind(&facts)
        }
        Mode::HtmlReport => {
            let input = read_stdin()?;
            let facts = design_fact_set_from_json(&input)?;
            let audit = design_audit(&facts);
            ValueString(design_html_report(&facts, &audit, None)).into_value()
        }
    };

    let json =
        serde_json::to_string(&output).map_err(|error| format!("failed to emit JSON: {error}"))?;
    println!("{json}");
    Ok(())
}

fn read_stdin() -> Result<String, String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    Ok(text)
}

fn print_usage() {
    println!(
        "Usage: design-check [--pack-payload [--parent-hash HASH] | --fixture-report | --css-static | --token-lint | --lower-css | --lower-tokens | --audit-facts | --drift-facts --baseline-json JSON | --tokens-dtcg | --tokens-tailwind | --html-report] [--tokens-json JSON]\n\nPack id: {PACK_ID}"
    );
}

struct ValueString(String);

impl ValueString {
    fn into_value(self) -> serde_json::Value {
        serde_json::Value::String(self.0)
    }
}

#[derive(Clone, Copy)]
enum Mode {
    PackPayload,
    FixtureReport,
    CssStatic,
    TokenLint,
    LowerCss,
    LowerTokens,
    AuditFacts,
    DriftFacts,
    TokensDtcg,
    TokensTailwind,
    HtmlReport,
}
