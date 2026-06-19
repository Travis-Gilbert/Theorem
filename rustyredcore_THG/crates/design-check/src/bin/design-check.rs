use design_check::{
    css_static_report, design_engineering_pack_payload, fixture_reports, lower_css,
    lower_tokens_json, token_lint_report, CssStaticInput, PACK_ID,
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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pack-payload" => mode = Mode::PackPayload,
            "--fixture-report" => mode = Mode::FixtureReport,
            "--css-static" => mode = Mode::CssStatic,
            "--token-lint" => mode = Mode::TokenLint,
            "--lower-css" => mode = Mode::LowerCss,
            "--lower-tokens" => mode = Mode::LowerTokens,
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
        "Usage: design-check [--pack-payload [--parent-hash HASH] | --fixture-report | --css-static | --token-lint | --lower-css | --lower-tokens] [--tokens-json JSON]\n\nPack id: {PACK_ID}"
    );
}

#[derive(Clone, Copy)]
enum Mode {
    PackPayload,
    FixtureReport,
    CssStatic,
    TokenLint,
    LowerCss,
    LowerTokens,
}
