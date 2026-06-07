use std::cell::RefCell;
use std::rc::Rc;

use lol_html::{element, HtmlRewriter, Settings};
use rustyred_thg_core::{GraphStore, GraphWriteResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::{
    apply_batch_to_store, build_v2_fixture_crawl, canonicalize_url, extract_links_for_url,
    global_robots_cache, CrawlBudget, CrawlRequest, CrawlScope, FetchCascade, FetchTierResult,
    FixturePage, RobotsDecision, RustyWebError, RustyWebResult,
};

pub type BrowserEngineResult<T> = Result<T, BrowserEngineError>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BrowserEngineError {
    NoCurrentPage,
    ElementNotFound { element_id: String },
    UnsupportedAction { reason: String },
    ActionBlocked { reason: String },
    RustyWeb { message: String },
}

impl From<RustyWebError> for BrowserEngineError {
    fn from(error: RustyWebError) -> Self {
        Self::RustyWeb {
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ElementBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InteractiveElement {
    pub element_id: String,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<ElementBox>,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub distilled_text: String,
    pub interactive_elements: Vec<InteractiveElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch: Option<FetchTierResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageExtract {
    pub url: String,
    pub title: String,
    pub text: String,
    pub links: Vec<String>,
    pub schema: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitCondition {
    LoadComplete,
    ElementVisible(String),
    Millis(u64),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BrowserAction {
    Click { element_id: String },
    Type { element_id: String, text: String },
    Select { element_id: String, value: String },
    Scroll { delta: i32 },
    Back,
    Forward,
    WaitFor { condition: WaitCondition },
    Submit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserActionPolicy {
    pub allow_state_changing: bool,
    pub confirmed: bool,
    pub require_robots: bool,
}

impl Default for BrowserActionPolicy {
    fn default() -> Self {
        Self {
            allow_state_changing: false,
            confirmed: false,
            require_robots: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowserActionOutcome {
    pub applied: bool,
    pub action: BrowserAction,
    pub page: PageState,
    pub receipt: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserPoolConfig {
    pub max_instances: usize,
    pub offload_target: String,
}

impl Default for BrowserPoolConfig {
    fn default() -> Self {
        Self {
            max_instances: 1,
            offload_target: "local_fetch_cascade".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FetchCascadeBrowserEngine {
    cascade: FetchCascade,
    max_bytes: usize,
    history: Vec<PageState>,
    history_index: usize,
}

impl FetchCascadeBrowserEngine {
    pub fn new(cascade: FetchCascade, max_bytes: usize) -> Self {
        Self {
            cascade,
            max_bytes,
            history: Vec::new(),
            history_index: 0,
        }
    }

    pub async fn navigate(&mut self, url: &str) -> BrowserEngineResult<PageState> {
        let canonical = canonicalize_url(url)?;
        let fetch = self
            .cascade
            .fetch_with_promotion(&canonical, self.max_bytes)
            .await?;
        let html = String::from_utf8_lossy(&fetch.html_bytes).to_string();
        let mut page = page_state_from_html(&fetch.final_url, &html)?;
        page.fetch = Some(fetch);
        if self.history_index + 1 < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(page.clone());
        self.history_index = self.history.len().saturating_sub(1);
        Ok(page)
    }

    pub fn observe(&self) -> BrowserEngineResult<PageState> {
        self.history
            .get(self.history_index)
            .cloned()
            .ok_or(BrowserEngineError::NoCurrentPage)
    }

    pub async fn act(
        &mut self,
        action: BrowserAction,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<BrowserActionOutcome> {
        if !action_allowed_by_policy(&action, policy) {
            return Err(BrowserEngineError::ActionBlocked {
                reason: "state-changing action requires allow_state_changing and confirmation"
                    .to_string(),
            });
        }
        let current = self.observe()?;
        if policy.require_robots && is_state_changing_action(&action) {
            let decision = global_robots_cache()
                .check(
                    self.cascade.client(),
                    &current.url,
                    self.cascade.user_agent(),
                )
                .await?;
            if !action_allowed_by_robots(&action, &decision) {
                return Err(BrowserEngineError::ActionBlocked {
                    reason: format!(
                        "robots.txt disallows a state-changing action on {}",
                        current.url
                    ),
                });
            }
        }
        let page = match &action {
            BrowserAction::Click { element_id } => {
                let element = current
                    .interactive_elements
                    .iter()
                    .find(|element| &element.element_id == element_id)
                    .ok_or_else(|| BrowserEngineError::ElementNotFound {
                        element_id: element_id.clone(),
                    })?;
                if element.role == "link" {
                    let Some(href) = &element.value else {
                        return Err(BrowserEngineError::UnsupportedAction {
                            reason: "link element has no href value".to_string(),
                        });
                    };
                    self.navigate(href).await?
                } else {
                    current.clone()
                }
            }
            BrowserAction::Type { element_id, text } => {
                let mut page = current.clone();
                let element = page
                    .interactive_elements
                    .iter_mut()
                    .find(|element| &element.element_id == element_id)
                    .ok_or_else(|| BrowserEngineError::ElementNotFound {
                        element_id: element_id.clone(),
                    })?;
                element.value = Some(text.clone());
                self.replace_current(page.clone());
                page
            }
            BrowserAction::Select { element_id, value } => {
                let mut page = current.clone();
                let element = page
                    .interactive_elements
                    .iter_mut()
                    .find(|element| &element.element_id == element_id)
                    .ok_or_else(|| BrowserEngineError::ElementNotFound {
                        element_id: element_id.clone(),
                    })?;
                element.value = Some(value.clone());
                self.replace_current(page.clone());
                page
            }
            BrowserAction::Scroll { .. } | BrowserAction::WaitFor { .. } => current.clone(),
            BrowserAction::Back => {
                if self.history_index > 0 {
                    self.history_index -= 1;
                }
                self.observe()?
            }
            BrowserAction::Forward => {
                if self.history_index + 1 < self.history.len() {
                    self.history_index += 1;
                }
                self.observe()?
            }
            BrowserAction::Submit => current.clone(),
        };
        Ok(BrowserActionOutcome {
            applied: true,
            action,
            page,
            receipt: json!({ "engine": "fetch_cascade_browser_engine" }),
        })
    }

    pub fn extract(&self, schema: Value) -> BrowserEngineResult<PageExtract> {
        let page = self.observe()?;
        let links = page
            .interactive_elements
            .iter()
            .filter(|element| element.role == "link")
            .filter_map(|element| element.value.clone())
            .collect();
        Ok(PageExtract {
            url: page.url,
            title: page.title,
            text: page.distilled_text,
            links,
            schema,
        })
    }

    fn replace_current(&mut self, page: PageState) {
        if let Some(slot) = self.history.get_mut(self.history_index) {
            *slot = page;
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WebConsumeRequest {
    pub run_id: String,
    pub url: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_ingest")]
    pub ingest: bool,
    #[serde(default = "default_respect_robots")]
    pub respect_robots: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebConsumeReceipt {
    pub run_id: String,
    pub url: String,
    pub page: PageState,
    pub extract: PageExtract,
    pub ingested: bool,
    pub writes: Vec<GraphWriteResult>,
    pub crawl_receipt: Option<crate::CrawlReceipt>,
}

pub async fn web_consume_to_graph<S: GraphStore>(
    store: &mut S,
    cascade: &FetchCascade,
    request: WebConsumeRequest,
) -> BrowserEngineResult<WebConsumeReceipt> {
    let canonical = canonicalize_url(&request.url)?;
    if request.respect_robots {
        let decision = global_robots_cache()
            .check(cascade.client(), &canonical, cascade.user_agent())
            .await?;
        if !decision.allowed {
            return Err(BrowserEngineError::ActionBlocked {
                reason: format!("robots.txt disallows fetching {canonical}"),
            });
        }
    }
    let fetch = cascade
        .fetch_with_promotion(&canonical, request.max_bytes)
        .await?;
    let html = String::from_utf8_lossy(&fetch.html_bytes).to_string();
    let mut page = page_state_from_html(&fetch.final_url, &html)?;
    page.fetch = Some(fetch.clone());
    let extract = PageExtract {
        url: page.url.clone(),
        title: page.title.clone(),
        text: page.distilled_text.clone(),
        links: extract_links_for_url(&page.url, &html).unwrap_or_default(),
        schema: json!({ "kind": "web_consume" }),
    };

    let (writes, crawl_receipt) = if request.ingest {
        let mut crawl_request = CrawlRequest::new(request.run_id.clone(), vec![page.url.clone()]);
        crawl_request.budget = CrawlBudget {
            max_pages: 1,
            max_seconds: 5,
            max_depth: 0,
            max_bytes: request.max_bytes,
        };
        crawl_request.scope = CrawlScope {
            namespace: request.namespace.clone(),
            follow_offsite: false,
            source_graph: "open_web_unverified".to_string(),
            source_license: "unknown".to_string(),
            federable: false,
            actor_id: request.actor_id.clone(),
            ..CrawlScope::default()
        };
        let output = build_v2_fixture_crawl(
            crawl_request,
            &[FixturePage {
                url: page.url.clone(),
                status: fetch.http_status,
                body: html,
                content_type: fetch.content_type.clone(),
                fetched_at: String::new(),
            }],
        )?;
        let writes = apply_batch_to_store(store, &output.graph.batch).map_err(|error| {
            BrowserEngineError::RustyWeb {
                message: format!("{}: {}", error.code, error.message),
            }
        })?;
        (writes, Some(output.receipt))
    } else {
        (Vec::new(), None)
    };

    Ok(WebConsumeReceipt {
        run_id: request.run_id,
        url: canonical,
        page,
        extract,
        ingested: request.ingest,
        writes,
        crawl_receipt,
    })
}

pub fn action_allowed_by_policy(action: &BrowserAction, policy: &BrowserActionPolicy) -> bool {
    if is_state_changing_action(action) {
        policy.allow_state_changing && policy.confirmed
    } else {
        true
    }
}

pub fn action_allowed_by_robots(action: &BrowserAction, decision: &RobotsDecision) -> bool {
    decision.allowed || !is_state_changing_action(action)
}

pub fn page_state_from_html(url: &str, html: &str) -> RustyWebResult<PageState> {
    let canonical = canonicalize_url(url)?;
    let title = extract_title(html);
    let distilled_text = html_to_text(html);
    let interactive_elements = extract_interactive_elements(&canonical, html)?;
    Ok(PageState {
        url: canonical,
        title,
        distilled_text,
        interactive_elements,
        fetch: None,
    })
}

fn extract_interactive_elements(
    base_url: &str,
    html: &str,
) -> RustyWebResult<Vec<InteractiveElement>> {
    let base = Url::parse(base_url).map_err(|err| RustyWebError::InvalidUrl {
        url: base_url.to_string(),
        reason: err.to_string(),
    })?;
    let elements = Rc::new(RefCell::new(Vec::<InteractiveElement>::new()));
    let links = Rc::clone(&elements);
    let buttons = Rc::clone(&elements);
    let inputs = Rc::clone(&elements);
    let selects = Rc::clone(&elements);
    let textareas = Rc::clone(&elements);

    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: vec![
                element!("a[href]", move |el| {
                    let mut joined = None;
                    if let Some(raw_href) = el.get_attribute("href") {
                        if let Ok(mut url) = base.join(&raw_href) {
                            url.set_fragment(None);
                            if matches!(url.scheme(), "http" | "https") {
                                joined = Some(url.to_string());
                            }
                        }
                    }
                    let mut borrowed = links.borrow_mut();
                    let index = borrowed.len();
                    borrowed.push(InteractiveElement {
                        element_id: format!("e{index}"),
                        role: "link".to_string(),
                        name: element_name(el, joined.as_deref().unwrap_or("link")),
                        value: joined,
                        bbox: None,
                        visible: !has_hidden_attribute(el),
                    });
                    Ok(())
                }),
                element!("button", move |el| {
                    let mut borrowed = buttons.borrow_mut();
                    let index = borrowed.len();
                    borrowed.push(InteractiveElement {
                        element_id: format!("e{index}"),
                        role: "button".to_string(),
                        name: element_name(el, "button"),
                        value: el.get_attribute("value"),
                        bbox: None,
                        visible: !has_hidden_attribute(el),
                    });
                    Ok(())
                }),
                element!("input", move |el| {
                    let mut borrowed = inputs.borrow_mut();
                    let index = borrowed.len();
                    let input_type = el
                        .get_attribute("type")
                        .unwrap_or_else(|| "text".to_string());
                    borrowed.push(InteractiveElement {
                        element_id: format!("e{index}"),
                        role: input_type,
                        name: element_name(el, "input"),
                        value: el.get_attribute("value"),
                        bbox: None,
                        visible: !has_hidden_attribute(el),
                    });
                    Ok(())
                }),
                element!("select", move |el| {
                    let mut borrowed = selects.borrow_mut();
                    let index = borrowed.len();
                    borrowed.push(InteractiveElement {
                        element_id: format!("e{index}"),
                        role: "select".to_string(),
                        name: element_name(el, "select"),
                        value: el.get_attribute("value"),
                        bbox: None,
                        visible: !has_hidden_attribute(el),
                    });
                    Ok(())
                }),
                element!("textarea", move |el| {
                    let mut borrowed = textareas.borrow_mut();
                    let index = borrowed.len();
                    borrowed.push(InteractiveElement {
                        element_id: format!("e{index}"),
                        role: "textbox".to_string(),
                        name: element_name(el, "textarea"),
                        value: None,
                        bbox: None,
                        visible: !has_hidden_attribute(el),
                    });
                    Ok(())
                }),
            ],
            ..Settings::default()
        },
        |_chunk: &[u8]| {},
    );
    rewriter
        .write(html.as_bytes())
        .map_err(|err| RustyWebError::HtmlParse {
            reason: err.to_string(),
        })?;
    rewriter.end().map_err(|err| RustyWebError::HtmlParse {
        reason: err.to_string(),
    })?;
    let extracted = elements.borrow().clone();
    Ok(extracted)
}

fn is_state_changing_action(action: &BrowserAction) -> bool {
    matches!(action, BrowserAction::Submit)
}

fn element_name(el: &lol_html::html_content::Element<'_, '_>, fallback: &str) -> String {
    el.get_attribute("aria-label")
        .or_else(|| el.get_attribute("title"))
        .or_else(|| el.get_attribute("name"))
        .or_else(|| el.get_attribute("id"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn has_hidden_attribute(el: &lol_html::html_content::Element<'_, '_>) -> bool {
    el.get_attribute("hidden").is_some()
        || el
            .get_attribute("aria-hidden")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

fn extract_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open_end) = lower[start..].find('>').map(|idx| start + idx + 1) else {
        return String::new();
    };
    let Some(close) = lower[open_end..].find("</title>").map(|idx| open_end + idx) else {
        return String::new();
    };
    html[open_end..close].trim().to_string()
}

fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn default_namespace() -> String {
    "open_web_unverified".to_string()
}

fn default_max_bytes() -> usize {
    5 * 1024 * 1024
}

fn default_ingest() -> bool {
    true
}

fn default_respect_robots() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FetchCascadeOptions, RobotsPolicyState};

    #[test]
    fn page_state_extracts_title_text_and_interactive_elements() {
        let page = page_state_from_html(
            "https://example.com/docs",
            r#"
            <html>
              <head><title>Docs</title></head>
              <body>
                <a href="/guide" aria-label="Guide link">Guide</a>
                <button id="save">Save</button>
                <input name="q" value="rust" />
              </body>
            </html>
            "#,
        )
        .expect("page");
        assert_eq!(page.title, "Docs");
        assert!(page.distilled_text.contains("Guide"));
        assert_eq!(page.interactive_elements.len(), 3);
        assert_eq!(
            page.interactive_elements[0].value.as_deref(),
            Some("https://example.com/guide")
        );
    }

    #[test]
    fn state_changing_actions_require_confirmation() {
        let policy = BrowserActionPolicy {
            allow_state_changing: true,
            confirmed: false,
            require_robots: true,
        };
        assert!(!action_allowed_by_policy(&BrowserAction::Submit, &policy));
        assert!(action_allowed_by_policy(
            &BrowserAction::Scroll { delta: 200 },
            &policy
        ));
    }

    #[test]
    fn robots_blocks_state_change_but_not_read_navigation() {
        let decision = RobotsDecision {
            allowed: false,
            state: RobotsPolicyState::Parsed,
            crawl_delay_seconds: None,
            sitemaps: Vec::new(),
            reason: "disallowed".to_string(),
        };
        assert!(!action_allowed_by_robots(&BrowserAction::Submit, &decision));
        assert!(action_allowed_by_robots(
            &BrowserAction::Click {
                element_id: "e0".to_string()
            },
            &decision
        ));
    }

    #[tokio::test]
    async fn engine_edits_local_form_state_without_network() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        let page = page_state_from_html(
            "https://example.com/",
            r#"<html><input name="q" value="" /></html>"#,
        )
        .expect("page");
        engine.history.push(page);
        let outcome = engine
            .act(
                BrowserAction::Type {
                    element_id: "e0".to_string(),
                    text: "servo".to_string(),
                },
                &BrowserActionPolicy::default(),
            )
            .await
            .expect("type");
        assert_eq!(
            outcome.page.interactive_elements[0].value.as_deref(),
            Some("servo")
        );
    }

    #[test]
    fn web_consume_request_defaults_respect_robots_true() {
        let request: WebConsumeRequest =
            serde_json::from_value(json!({ "run_id": "r1", "url": "https://example.com/" }))
                .expect("deserialize");
        assert!(request.respect_robots);
    }
}
