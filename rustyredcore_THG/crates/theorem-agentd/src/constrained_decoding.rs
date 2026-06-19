//! Constrained decoding for the Burn serving path (CHK020).
//!
//! burn-lm does not ship structured output, so when the daemon's model runs on the
//! Burn path the sampler itself must enforce the tool-call grammar. This module
//! compiles the `ToolCatalog` into a token-level logit mask: at each decode step,
//! given the text generated so far, it returns which vocabulary tokens keep the
//! output a viable prefix of a valid envelope. The sampler sets the logits of the
//! disallowed tokens to negative infinity, so even a small local model cannot emit
//! an off-grammar tool call.
//!
//! The grammar is exactly the daemon's envelope (the same shape `tools.rs`
//! enforces with GBNF on the llama-server path):
//!
//! - `{"type":"tool_call","name":"<one of the catalog tool names>","arguments":<object>}`
//! - `{"type":"final","text":"<string>"}`
//!
//! The catalog-specific compiled artifact is the set of legal tool names; the rest
//! of the envelope is fixed structure plus generic JSON for the value slots. This
//! is the model/weight/GPU-free half of CHK020; wiring `token_mask` into the Burn
//! sampler loop is the remaining step once the Burn server lands.

use crate::tools::ToolCatalog;

/// A decoder vocabulary: a fixed set of tokens, each mapping to the UTF-8 piece it
/// appends to the output. Implemented over whatever tokenizer the Burn model uses.
pub trait Vocab {
    /// Number of tokens in the vocabulary.
    fn len(&self) -> usize;
    /// Whether the vocabulary contains no tokens.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The string piece token `id` appends (may be empty for control tokens).
    fn piece(&self, id: usize) -> &str;
}

/// The compiled tool-call grammar: the fixed envelope plus the catalog's tool names.
#[derive(Clone, Debug)]
pub struct ToolGrammar {
    tool_names: Vec<String>,
}

impl ToolGrammar {
    /// Compile the grammar from the tool catalog (CHK020: "compiling the tools.rs
    /// tool catalog into a token-level logit mask").
    pub fn from_catalog(catalog: &ToolCatalog) -> Self {
        let mut tool_names: Vec<String> = catalog.tools().map(|tool| tool.name.clone()).collect();
        tool_names.sort();
        tool_names.dedup();
        Self { tool_names }
    }

    /// The legal tool names (the catalog-specific compiled artifact).
    pub fn tool_names(&self) -> &[String] {
        &self.tool_names
    }

    /// True when `decoded` is a viable prefix of some valid envelope: either it can
    /// still be extended to a valid envelope, or it already is one.
    pub fn viable_prefix(&self, decoded: &str) -> bool {
        envelope_viable(decoded, &self.tool_names)
    }

    /// The logit mask for the next token: `mask[id]` is true when appending
    /// `vocab.piece(id)` to `decoded` keeps the output a viable prefix. A token
    /// with an empty piece (e.g. a control token) is always allowed; the sampler
    /// decides separately when to stop.
    pub fn token_mask<V: Vocab>(&self, decoded: &str, vocab: &V) -> Vec<bool> {
        let mut scratch = String::with_capacity(decoded.len() + 16);
        (0..vocab.len())
            .map(|id| {
                let piece = vocab.piece(id);
                if piece.is_empty() {
                    return true;
                }
                scratch.clear();
                scratch.push_str(decoded);
                scratch.push_str(piece);
                self.viable_prefix(&scratch)
            })
            .collect()
    }
}

/// Whether `s` is a viable prefix of a valid envelope given the legal tool names.
fn envelope_viable(s: &str, tool_names: &[String]) -> bool {
    let bytes = s.as_bytes();
    let mut cur = Cursor::new(bytes);

    // `{`
    match cur.expect_byte(b'{') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    // `"type"`
    match cur.expect_literal("\"type\"") {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_byte(b':') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    // The type value: a quoted string that must prefix-match "tool_call" or "final".
    match cur.expect_enum_string(&["tool_call", "final"]) {
        EnumStep::Partial => true,
        EnumStep::Reject => false,
        EnumStep::Ok(value) => match value.as_str() {
            "tool_call" => tool_call_tail(&mut cur, tool_names),
            "final" => final_tail(&mut cur),
            _ => false,
        },
    }
}

/// Match the tool_call branch tail after the type value:
/// `,"name":"<NAME>","arguments":<object>}`.
fn tool_call_tail(cur: &mut Cursor, tool_names: &[String]) -> bool {
    match cur.expect_byte(b',') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_literal("\"name\"") {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_byte(b':') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    let names: Vec<&str> = tool_names.iter().map(String::as_str).collect();
    match cur.expect_enum_string(&names) {
        EnumStep::Partial => return true,
        EnumStep::Reject => return false,
        EnumStep::Ok(_) => {}
    }
    match cur.expect_byte(b',') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_literal("\"arguments\"") {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_byte(b':') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    // A JSON object value.
    match cur.expect_json_object() {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_byte(b'}') {
        Step::Partial => true,
        Step::Reject => false,
        // A complete, valid envelope, with nothing trailing.
        Step::Ok => cur.at_end(),
    }
}

/// Match the final branch tail after the type value: `,"text":"<string>"}`.
fn final_tail(cur: &mut Cursor) -> bool {
    match cur.expect_byte(b',') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_literal("\"text\"") {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_byte(b':') {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_json_string() {
        Step::Partial => return true,
        Step::Reject => return false,
        Step::Ok => {}
    }
    match cur.expect_byte(b'}') {
        Step::Partial => true,
        Step::Reject => false,
        Step::Ok => cur.at_end(),
    }
}

/// Outcome of consuming a fixed token: matched fully, input ended mid-token (still
/// viable), or a definite mismatch.
enum Step {
    Ok,
    Partial,
    Reject,
}

/// Outcome of consuming an enumerated quoted string.
enum EnumStep {
    Ok(String),
    Partial,
    Reject,
}

/// A byte cursor with whitespace-skipping, fixed-literal, and JSON helpers. Every
/// method treats running out of input mid-construct as `Partial` (viable), so the
/// acceptor never rejects a string that could still be completed.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn skip_ws(&mut self) -> bool {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => return true,
            }
        }
        false
    }

    /// Expect a single non-string byte (with surrounding whitespace allowed).
    fn expect_byte(&mut self, want: u8) -> Step {
        if !self.skip_ws() {
            return Step::Partial;
        }
        if self.bytes[self.pos] == want {
            self.pos += 1;
            Step::Ok
        } else {
            Step::Reject
        }
    }

    /// Expect a fixed literal (e.g. `"type"`), whitespace allowed before it.
    fn expect_literal(&mut self, literal: &str) -> Step {
        if !self.skip_ws() {
            return Step::Partial;
        }
        let lit = literal.as_bytes();
        let mut i = 0;
        while i < lit.len() {
            if self.pos + i >= self.bytes.len() {
                // Input ended partway through the literal: still viable.
                self.pos = self.bytes.len();
                return Step::Partial;
            }
            if self.bytes[self.pos + i] != lit[i] {
                return Step::Reject;
            }
            i += 1;
        }
        self.pos += lit.len();
        Step::Ok
    }

    /// Expect a quoted string whose contents must prefix-match one of `options`.
    fn expect_enum_string(&mut self, options: &[&str]) -> EnumStep {
        if !self.skip_ws() {
            return EnumStep::Partial;
        }
        if self.bytes[self.pos] != b'"' {
            return EnumStep::Reject;
        }
        let mut i = self.pos + 1;
        let mut value = String::new();
        loop {
            if i >= self.bytes.len() {
                // Open string: viable iff some option still has `value` as a prefix.
                return if options.iter().any(|opt| opt.starts_with(&value)) {
                    EnumStep::Partial
                } else {
                    EnumStep::Reject
                };
            }
            let b = self.bytes[i];
            if b == b'"' {
                // Closed string: must equal one of the options exactly.
                if options.iter().any(|opt| *opt == value) {
                    self.pos = i + 1;
                    return EnumStep::Ok(value);
                }
                return EnumStep::Reject;
            }
            // Tool names and the type values are plain ASCII (no escapes needed).
            value.push(b as char);
            if !options.iter().any(|opt| opt.starts_with(&value)) {
                return EnumStep::Reject;
            }
            i += 1;
        }
    }

    /// Expect a JSON string value (escape-aware), whitespace allowed before it.
    fn expect_json_string(&mut self) -> Step {
        if !self.skip_ws() {
            return Step::Partial;
        }
        if self.bytes[self.pos] != b'"' {
            return Step::Reject;
        }
        let mut i = self.pos + 1;
        let mut escaped = false;
        while i < self.bytes.len() {
            let b = self.bytes[i];
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                self.pos = i + 1;
                return Step::Ok;
            }
            i += 1;
        }
        // Unterminated string: viable (could still close).
        self.pos = self.bytes.len();
        Step::Partial
    }

    /// Expect a JSON object value (balanced, escape/string-aware), whitespace
    /// allowed before it. Returns `Ok` only when the object is fully balanced.
    fn expect_json_object(&mut self) -> Step {
        if !self.skip_ws() {
            return Step::Partial;
        }
        if self.bytes[self.pos] != b'{' {
            return Step::Reject;
        }
        let mut i = self.pos;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        while i < self.bytes.len() {
            let b = self.bytes[i];
            if in_string {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
            } else {
                match b {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            self.pos = i + 1;
                            return Step::Ok;
                        }
                        if depth < 0 {
                            return Step::Reject;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        // Ran out of input with the object still open: viable.
        self.pos = self.bytes.len();
        Step::Partial
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grammar() -> ToolGrammar {
        ToolGrammar::from_catalog(&ToolCatalog::default_catalog())
    }

    #[test]
    fn compiles_tool_names_from_catalog() {
        let grammar = grammar();
        assert!(grammar.tool_names().iter().any(|n| n == "coordinate"));
        assert!(grammar.tool_names().iter().any(|n| n == "job_submit"));
        assert!(grammar
            .tool_names()
            .iter()
            .any(|n| n == "ticktick_update_task"));
    }

    #[test]
    fn accepts_a_complete_tool_call_envelope() {
        let g = grammar();
        assert!(g.viable_prefix(
            r#"{"type":"tool_call","name":"job_submit","arguments":{"title":"x","repo":"r"}}"#
        ));
    }

    #[test]
    fn accepts_a_complete_final_envelope() {
        let g = grammar();
        assert!(g.viable_prefix(r#"{"type":"final","text":"all done"}"#));
    }

    #[test]
    fn accepts_growing_prefixes() {
        let g = grammar();
        let full = r#"{"type":"tool_call","name":"coordinate","arguments":{}}"#;
        // Every prefix of a valid envelope must itself be viable.
        for end in 1..=full.len() {
            assert!(
                g.viable_prefix(&full[..end]),
                "prefix should be viable: {:?}",
                &full[..end]
            );
        }
    }

    #[test]
    fn rejects_unknown_tool_name() {
        let g = grammar();
        // A name that is not a prefix of any catalog name is rejected as soon as it
        // diverges.
        assert!(!g.viable_prefix(r#"{"type":"tool_call","name":"made_up"#));
        // A valid prefix of a real name is still viable.
        assert!(g.viable_prefix(r#"{"type":"tool_call","name":"job_su"#));
    }

    #[test]
    fn rejects_wrong_structure() {
        let g = grammar();
        assert!(!g.viable_prefix(r#"{"kind":"#)); // wrong first key
        assert!(!g.viable_prefix(r#"{"type":"tool_call","args""#)); // wrong second key
        assert!(!g.viable_prefix("[")); // not an object
    }

    #[test]
    fn rejects_trailing_garbage_after_complete_envelope() {
        let g = grammar();
        assert!(!g.viable_prefix(r#"{"type":"final","text":"x"} and more"#));
    }

    #[test]
    fn type_value_must_prefix_match_a_branch() {
        let g = grammar();
        assert!(g.viable_prefix(r#"{"type":"too"#)); // prefix of tool_call
        assert!(g.viable_prefix(r#"{"type":"fin"#)); // prefix of final
        assert!(!g.viable_prefix(r#"{"type":"xyz"#)); // neither
    }

    // A toy byte-per-token vocab so the mask projection is testable without a real
    // tokenizer.
    struct ByteVocab {
        pieces: Vec<String>,
    }

    impl ByteVocab {
        fn new() -> Self {
            // One token per printable ASCII byte plus a couple of multi-char pieces.
            let mut pieces: Vec<String> =
                (0x20u8..=0x7e).map(|b| (b as char).to_string()).collect();
            pieces.push("tool_call".to_string());
            pieces.push("".to_string()); // a control/stop token
            Self { pieces }
        }
    }

    impl Vocab for ByteVocab {
        fn len(&self) -> usize {
            self.pieces.len()
        }
        fn piece(&self, id: usize) -> &str {
            &self.pieces[id]
        }
    }

    #[test]
    fn token_mask_allows_only_grammar_continuations() {
        let g = grammar();
        let vocab = ByteVocab::new();
        // After the opening brace, only a quote (start of "type") may follow, plus
        // whitespace and the always-allowed empty control token.
        let mask = g.token_mask("{", &vocab);
        for (id, allowed) in mask.iter().enumerate() {
            let piece = vocab.piece(id);
            let expected =
                piece.is_empty() || piece.chars().all(|c| c.is_ascii_whitespace()) || piece == "\"";
            assert_eq!(
                *allowed, expected,
                "piece {piece:?} allow-state should be {expected}"
            );
        }
    }

    #[test]
    fn token_mask_blocks_a_bad_first_byte() {
        let g = grammar();
        let vocab = ByteVocab::new();
        // From empty, only '{' (and whitespace / empty) may start the envelope.
        let mask = g.token_mask("", &vocab);
        let brace = vocab.pieces.iter().position(|p| p == "{").unwrap();
        let letter = vocab.pieces.iter().position(|p| p == "a").unwrap();
        assert!(mask[brace]);
        assert!(!mask[letter]);
    }
}
