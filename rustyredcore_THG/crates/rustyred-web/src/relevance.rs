//! Relevance extraction: turn a fetched HTML page into the few passages that
//! actually answer a query.
//!
//! This is the "scrape the relevant pieces" half of the progressive-disclosure
//! search path. A provider snippet is whatever the search engine chose to show
//! for *its* interpretation of the query; this module instead extracts the
//! page's own main content and ranks its passages against the *user's* query, so
//! the excerpt is query-aligned and under our control.
//!
//! Pipeline: [`extract_main_text`] (lol_html boilerplate strip) ->
//! [`split_passages`] -> a [`PassageScorer`] (lexical now, embedding-upgradeable)
//! -> top-k via [`relevant_excerpt`].
//!
//! Pure and deterministic: no network, no clock, stable tie-breaks. The cut
//! point and ranking are reproducible so the parity gates stay green.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use lol_html::html_content::ContentType;
use lol_html::{element, HtmlRewriter, Settings};

/// Tags whose entire subtree is boilerplate/non-content and is dropped before
/// text extraction. Without this, passage ranking would score nav menus, cookie
/// banners, and inline script/style as if they were article text.
const BOILERPLATE_SELECTOR: &str =
    "script, style, noscript, template, svg, nav, header, footer, aside, form, button, iframe";

/// Block-level tags after which a paragraph break is inserted, so the flattened
/// text splits into passages on real content boundaries rather than mid-sentence.
const BLOCK_SELECTOR: &str =
    "p, div, section, article, li, br, h1, h2, h3, h4, h5, h6, blockquote, tr, pre, hr, td";

/// Minimum size for a kept passage. Drops nav crumbs, single-word fragments, and
/// blank lines that survive flattening.
const MIN_PASSAGE_CHARS: usize = 24;
const MIN_PASSAGE_WORDS: usize = 4;

/// tf saturation constant for the lexical scorer (BM25's `k1`). Higher means
/// repeated occurrences of a term keep adding; lower means a single hit nearly
/// saturates.
const K1: f64 = 1.2;

/// Sentinel inserted after each block element (see `clean_main_html`) to mark a
/// passage boundary. Chosen as ASCII Record Separator so it never collides with
/// real page text -- this distinguishes true block breaks from the incidental
/// source-formatting newlines that HTML routinely places inside a block.
const BLOCK_SEP: char = '\u{1e}';

/// A passage of page text with its relevance score for a query. Higher score is
/// more relevant.
#[derive(Clone, Debug, PartialEq)]
pub struct Passage {
    pub text: String,
    pub score: f64,
}

/// Scores page passages against a query. Implemented lexically now; an
/// embedding-backed scorer can replace it later behind the same trait so the
/// extraction pipeline is unchanged. Mirrors the `ConnectionScorer` pattern in
/// `epistemic_filter.rs`.
pub trait PassageScorer {
    /// Return one score per passage, in the same order as `passages`. Higher is
    /// more relevant; `0.0` means "no signal".
    fn score(&self, query: &str, passages: &[String]) -> Vec<f64>;
}

/// Deterministic lexical scorer: distinct-query-term coverage with tf saturation
/// and mild length normalization (a compact, single-document BM25 flavor). No
/// model, no network -- the cold-start default and the parity baseline. Swap in
/// an embedding scorer later for semantic (paraphrase/synonym) matching.
#[derive(Clone, Debug, Default)]
pub struct LexicalScorer;

impl PassageScorer for LexicalScorer {
    fn score(&self, query: &str, passages: &[String]) -> Vec<f64> {
        let mut query_terms = tokenize(query);
        query_terms.sort();
        query_terms.dedup();
        if query_terms.is_empty() {
            return vec![0.0; passages.len()];
        }
        passages
            .iter()
            .map(|passage| score_passage(&query_terms, passage))
            .collect()
    }
}

fn score_passage(query_terms: &[String], passage: &str) -> f64 {
    let tokens = tokenize(passage);
    if tokens.is_empty() {
        return 0.0;
    }
    let mut term_freq: BTreeMap<&str, u32> = BTreeMap::new();
    for token in &tokens {
        *term_freq.entry(token.as_str()).or_insert(0) += 1;
    }
    let mut matched = 0u32;
    let mut acc = 0.0f64;
    for term in query_terms {
        if let Some(&freq) = term_freq.get(term.as_str()) {
            matched += 1;
            let freq = freq as f64;
            acc += freq / (freq + K1); // tf saturation in (0, 1)
        }
    }
    if matched == 0 {
        return 0.0;
    }
    // Reward passages that touch more of the distinct query, not ones that
    // repeat a single term.
    let coverage = matched as f64 / query_terms.len() as f64;
    // Mild length normalization so a long passage cannot win on size alone.
    let length_norm = 1.0 / (1.0 + (tokens.len() as f64 / 240.0));
    acc * coverage * length_norm
}

/// Extract the page's main text, with boilerplate (script/style/nav/header/
/// footer/aside/...) removed and block boundaries preserved as newlines.
pub fn extract_main_text(html: &str) -> String {
    let cleaned = clean_main_html(html);
    let raw = strip_tags_keep_breaks(&cleaned);
    // Split on the block sentinel (not raw newlines: HTML routinely wraps text
    // mid-paragraph), collapse each block's internal whitespace, and rejoin one
    // block per line.
    raw.split(BLOCK_SEP)
        .map(normalize_whitespace)
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Split flattened page text into candidate passages, dropping fragments too
/// small to be useful.
pub fn split_passages(text: &str) -> Vec<String> {
    text.split('\n')
        .map(normalize_whitespace)
        .filter(|passage| {
            passage.chars().count() >= MIN_PASSAGE_CHARS
                && passage.split_whitespace().count() >= MIN_PASSAGE_WORDS
        })
        .collect()
}

/// Extract the top relevant passages from a page for a query.
///
/// Returns at most `max_passages` passages, bounded by `max_chars` total, ranked
/// by `scorer`. Passages with zero score are excluded -- an empty result means
/// the page had no query-relevant content (caller should fall back to the
/// provider snippet). Ordering is deterministic: score descending, then original
/// document order as the tie-break.
pub fn relevant_excerpt(
    query: &str,
    html: &str,
    max_passages: usize,
    max_chars: usize,
    scorer: &dyn PassageScorer,
) -> Vec<Passage> {
    let passages = split_passages(&extract_main_text(html));
    if passages.is_empty() || max_passages == 0 {
        return Vec::new();
    }
    let scores = scorer.score(query, &passages);

    let mut ranked: Vec<(usize, Passage)> = passages
        .into_iter()
        .zip(scores)
        .map(|(text, score)| Passage { text, score })
        .enumerate()
        .collect();
    ranked.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut excerpt = Vec::new();
    let mut used_chars = 0usize;
    for (_, passage) in ranked {
        if excerpt.len() >= max_passages || passage.score <= 0.0 {
            break;
        }
        let len = passage.text.chars().count();
        if !excerpt.is_empty() && used_chars + len > max_chars {
            break;
        }
        used_chars += len;
        excerpt.push(passage);
    }
    excerpt
}

/// Convenience wrapper over [`relevant_excerpt`] using the default
/// [`LexicalScorer`].
pub fn relevant_excerpt_lexical(
    query: &str,
    html: &str,
    max_passages: usize,
    max_chars: usize,
) -> Vec<Passage> {
    relevant_excerpt(query, html, max_passages, max_chars, &LexicalScorer)
}

/// Remove boilerplate subtrees and insert a newline after each block element,
/// returning the cleaned HTML. Falls back to the input unchanged if the rewriter
/// errors on malformed markup.
fn clean_main_html(html: &str) -> String {
    let output: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::with_capacity(html.len())));
    let sink = Rc::clone(&output);
    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![
                element!(BOILERPLATE_SELECTOR, |el| {
                    el.remove();
                    Ok(())
                }),
                element!(BLOCK_SELECTOR, |el| {
                    // Mark a passage boundary with the sentinel, not a newline:
                    // raw newlines inside a block would otherwise fragment it.
                    el.after("\u{1e}", ContentType::Text);
                    Ok(())
                }),
            ],
            ..Settings::default()
        },
        move |chunk: &[u8]| sink.borrow_mut().extend_from_slice(chunk),
    );
    if rewriter.write(html.as_bytes()).is_err() || rewriter.end().is_err() {
        return html.to_string();
    }
    let bytes = output.borrow();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Strip remaining (inline) tags while preserving the newlines inserted at block
/// boundaries, then decode common HTML entities.
fn strip_tags_keep_breaks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if in_tag => {}
            _ => out.push(ch),
        }
    }
    decode_entities(&out)
}

fn normalize_whitespace(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lowercase alphanumeric tokens. Unicode-aware via `char::is_alphanumeric`.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Decode the handful of HTML entities common in extracted text. `&amp;` is
/// decoded last so an already-encoded entity is not double-decoded.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <html><head>
            <style>.banner{color:red}</style>
            <script>var secret = "do not index me";</script>
        </head><body>
            <nav><a href="/">Home</a><a href="/about">About Us</a></nav>
            <article>
                <h1>Heading</h1>
                <p>The Tokio runtime is an asynchronous runtime for the Rust
                   programming language, providing the building blocks needed for
                   writing network applications.</p>
                <p>This second paragraph is entirely about cooking pasta with
                   fresh tomatoes and basil, and has nothing whatsoever to do
                   with software.</p>
            </article>
            <footer>Copyright 2026 Example Incorporated, all rights reserved.</footer>
        </body></html>
    "#;

    #[test]
    fn extract_main_text_drops_boilerplate_and_keeps_content() {
        let text = extract_main_text(PAGE);
        assert!(text.contains("asynchronous runtime"));
        assert!(text.contains("cooking pasta"));
        // script / style / nav / footer content is gone.
        assert!(!text.contains("do not index me"));
        assert!(!text.contains("color:red"));
        assert!(!text.contains("About Us"));
        assert!(!text.contains("Copyright"));
    }

    #[test]
    fn split_passages_drops_tiny_fragments() {
        let passages = split_passages(&extract_main_text(PAGE));
        // The two content paragraphs survive; the short <h1> "Heading" does not.
        assert!(passages.iter().any(|p| p.contains("Tokio runtime")));
        assert!(passages.iter().any(|p| p.contains("cooking pasta")));
        assert!(passages
            .iter()
            .all(|p| !p.trim().eq_ignore_ascii_case("Heading")));
    }

    #[test]
    fn lexical_scorer_ranks_query_relevant_passage_first() {
        let excerpt = relevant_excerpt_lexical("tokio async runtime rust", PAGE, 1, 1000);
        assert_eq!(excerpt.len(), 1);
        assert!(excerpt[0].text.contains("asynchronous runtime"));
        assert!(excerpt[0].score > 0.0);
    }

    #[test]
    fn a_different_query_selects_a_different_passage() {
        let excerpt = relevant_excerpt_lexical("pasta tomatoes basil", PAGE, 1, 1000);
        assert_eq!(excerpt.len(), 1);
        assert!(excerpt[0].text.contains("cooking pasta"));
    }

    #[test]
    fn no_relevant_content_yields_empty_excerpt() {
        // No page passage matches -> empty, so the caller falls back to the snippet.
        let excerpt = relevant_excerpt_lexical("quantum chromodynamics gluon", PAGE, 3, 1000);
        assert!(excerpt.is_empty());
    }

    #[test]
    fn max_passages_and_char_budget_are_respected() {
        let one = relevant_excerpt_lexical("runtime pasta rust tomatoes", PAGE, 1, 10_000);
        assert_eq!(one.len(), 1);
        // A tiny char budget still returns at least the single top passage.
        let tight = relevant_excerpt_lexical("runtime pasta rust tomatoes", PAGE, 5, 1);
        assert_eq!(tight.len(), 1);
    }

    #[test]
    fn ranking_is_deterministic() {
        let a = relevant_excerpt_lexical("tokio runtime rust", PAGE, 3, 1000);
        let b = relevant_excerpt_lexical("tokio runtime rust", PAGE, 3, 1000);
        assert_eq!(a, b);
    }

    #[test]
    fn empty_query_scores_zero() {
        let scores = LexicalScorer.score("", &["some passage of text here".to_string()]);
        assert_eq!(scores, vec![0.0]);
    }
}
