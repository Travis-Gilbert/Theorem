use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use lol_html::{element, HtmlRewriter, Settings};
use rustyred_thg_core::{GraphStore, GraphWriteResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::browser_perception::{
    extract_structured, keyboard_fallback_for, resolve_upload_path, DomainPolicy,
    NavigationDecision, SensitiveData, TabSet, UploadDecision,
};
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
    /// job-007 D5: the engine has not yet rolled a proper interactive role or
    /// bounds for this node, so it is surfaced degraded. A degraded element is
    /// still operable (keyboard fallback), but the driving model should prefer
    /// keyboard or vision over a precise click. Additive; serde-defaults false
    /// so older receipts and the HTML reader path round-trip unchanged.
    #[serde(default)]
    pub degraded: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub distilled_text: String,
    pub interactive_elements: Vec<InteractiveElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab_id: Option<String>,
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
    #[serde(default)]
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
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
    Click {
        element_id: String,
    },
    Type {
        element_id: String,
        text: String,
    },
    Select {
        element_id: String,
        value: String,
    },
    SendKeys {
        sequence: String,
    },
    SelectOption {
        element_id: String,
        value: String,
    },
    Scroll {
        delta: i32,
    },
    ScrollToElement {
        element_id: String,
    },
    UploadFile {
        element_id: String,
        path: String,
    },
    Back,
    Forward,
    WaitFor {
        condition: WaitCondition,
    },
    Submit,
    OpenTab {
        url: String,
    },
    SwitchTab {
        tab_id: String,
    },
    CloseTab {
        tab_id: String,
    },
    ListTabs,
    Extract {
        schema: Value,
        candidate: Value,
    },
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowserActionPolicy {
    pub allow_state_changing: bool,
    pub confirmed: bool,
    pub require_robots: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permitted_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upload_roots: Vec<String>,
    #[serde(default)]
    pub sensitive_data: SensitiveData,
}

impl Default for BrowserActionPolicy {
    fn default() -> Self {
        Self {
            allow_state_changing: false,
            confirmed: false,
            require_robots: true,
            permitted_domains: Vec::new(),
            upload_roots: Vec::new(),
            sensitive_data: SensitiveData::default(),
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
    tabs: TabSet,
    tab_pages: BTreeMap<String, PageState>,
}

impl FetchCascadeBrowserEngine {
    pub fn new(cascade: FetchCascade, max_bytes: usize) -> Self {
        Self {
            cascade,
            max_bytes,
            history: Vec::new(),
            history_index: 0,
            tabs: TabSet::new(),
            tab_pages: BTreeMap::new(),
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
        self.stamp_active_tab(&mut page);
        if self.history_index + 1 < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(page.clone());
        self.history_index = self.history.len().saturating_sub(1);
        self.store_active_tab_page(&page);
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
        enforce_domain_policy(&action, &current, policy)?;
        let domain = domain_for_sensitive_data(&current.url);
        let mut receipt = json!({
            "engine": "fetch_cascade_browser_engine",
            "executor": "fetch_cascade_fallback",
            "accesskit_action": "accesskit_action_request_pending"
        });
        let page = match &action {
            BrowserAction::Click { element_id } => {
                let element = current
                    .interactive_elements
                    .iter()
                    .find(|element| &element.element_id == element_id)
                    .ok_or_else(|| BrowserEngineError::ElementNotFound {
                        element_id: element_id.clone(),
                    })?;
                if let Some(fallback) = keyboard_fallback_for(element) {
                    receipt = json!({
                        "engine": "fetch_cascade_browser_engine",
                        "executor": "keyboard_fallback",
                        "accesskit_action": "accesskit_action_request_pending",
                        "keyboard_fallback": fallback
                    });
                }
                if element.role == "link" {
                    let Some(href) = &element.value else {
                        return Ok(BrowserActionOutcome {
                            applied: true,
                            action,
                            page: current.clone(),
                            receipt,
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
                let resolved = policy.sensitive_data.resolve_placeholders(&domain, text);
                let masked = policy.sensitive_data.mask(&domain, &resolved);
                element.value = Some(masked.masked.clone());
                receipt = json!({
                    "engine": "fetch_cascade_browser_engine",
                    "executor": "local_value_update",
                    "masked_text": masked.masked,
                    "sensitive_keys": masked.used_keys
                });
                self.replace_current(page.clone());
                page
            }
            BrowserAction::Select { element_id, value } => {
                self.apply_select(element_id, value, &domain, policy, &mut receipt)?
            }
            BrowserAction::SelectOption { element_id, value } => {
                self.apply_select(element_id, value, &domain, policy, &mut receipt)?
            }
            BrowserAction::SendKeys { sequence } => {
                let resolved = policy
                    .sensitive_data
                    .resolve_placeholders(&domain, sequence);
                let masked = policy.sensitive_data.mask(&domain, &resolved);
                receipt = json!({
                    "engine": "fetch_cascade_browser_engine",
                    "executor": "keyboard_fallback",
                    "masked_sequence": masked.masked,
                    "sensitive_keys": masked.used_keys,
                    "accesskit_action": "accesskit_action_request_pending"
                });
                current.clone()
            }
            BrowserAction::UploadFile { element_id, path } => {
                let mut page = current.clone();
                let element = page
                    .interactive_elements
                    .iter_mut()
                    .find(|element| &element.element_id == element_id)
                    .ok_or_else(|| BrowserEngineError::ElementNotFound {
                        element_id: element_id.clone(),
                    })?;
                match resolve_upload_path(path, &policy.upload_roots) {
                    UploadDecision::Allowed { path } => {
                        let masked = policy.sensitive_data.mask(&domain, &path);
                        element.value = Some(masked.masked.clone());
                        receipt = json!({
                            "engine": "fetch_cascade_browser_engine",
                            "executor": "file_input_allowlisted",
                            "upload_path": masked.masked,
                            "sensitive_keys": masked.used_keys,
                            "accesskit_action": "accesskit_action_request_pending"
                        });
                    }
                    UploadDecision::Refused { reason } => {
                        return Err(BrowserEngineError::ActionBlocked { reason });
                    }
                }
                self.replace_current(page.clone());
                page
            }
            BrowserAction::Scroll { .. } | BrowserAction::WaitFor { .. } => current.clone(),
            BrowserAction::ScrollToElement { element_id } => {
                let element = current
                    .interactive_elements
                    .iter()
                    .find(|element| &element.element_id == element_id)
                    .ok_or_else(|| BrowserEngineError::ElementNotFound {
                        element_id: element_id.clone(),
                    })?;
                receipt = json!({
                    "engine": "fetch_cascade_browser_engine",
                    "executor": "scroll_to_element",
                    "element_id": element.element_id,
                    "bbox": element.bbox,
                    "degraded": element.degraded,
                    "accesskit_action": "accesskit_action_request_pending"
                });
                current.clone()
            }
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
            BrowserAction::OpenTab { url } => {
                self.tabs.open(url.clone());
                self.navigate(url).await?
            }
            BrowserAction::SwitchTab { tab_id } => {
                self.tabs
                    .switch(tab_id)
                    .map_err(|reason| BrowserEngineError::ActionBlocked { reason })?;
                if let Some(page) = self.tab_pages.get(tab_id).cloned() {
                    self.replace_current(page.clone());
                    page
                } else {
                    let mut page = current.clone();
                    page.active_tab_id = Some(tab_id.clone());
                    self.replace_current(page.clone());
                    page
                }
            }
            BrowserAction::CloseTab { tab_id } => {
                self.tabs
                    .close(tab_id)
                    .map_err(|reason| BrowserEngineError::ActionBlocked { reason })?;
                let mut page = self
                    .tabs
                    .active()
                    .and_then(|tab| self.tab_pages.get(&tab.id).cloned())
                    .unwrap_or_else(|| current.clone());
                page.active_tab_id = self.tabs.active().map(|tab| tab.id.clone());
                self.replace_current(page.clone());
                page
            }
            BrowserAction::ListTabs => {
                receipt = json!({
                    "engine": "fetch_cascade_browser_engine",
                    "tabs": self.tabs.list()
                });
                current.clone()
            }
            BrowserAction::Extract { schema, candidate } => {
                let outcome = extract_structured(&current, candidate.clone(), schema.clone());
                receipt = json!({
                    "engine": "fetch_cascade_browser_engine",
                    "extract": outcome
                });
                current.clone()
            }
            BrowserAction::Done { summary } => {
                receipt = json!({
                    "engine": "fetch_cascade_browser_engine",
                    "done": true,
                    "summary": summary
                });
                current.clone()
            }
        };
        Ok(BrowserActionOutcome {
            applied: true,
            action,
            page,
            receipt,
        })
    }

    pub fn extract(&self, schema: Value) -> BrowserEngineResult<PageExtract> {
        let page = self.observe()?;
        let links: Vec<String> = page
            .interactive_elements
            .iter()
            .filter(|element| element.role == "link")
            .filter_map(|element| element.value.clone())
            .collect();
        let candidate = json!({
            "url": page.url.clone(),
            "title": page.title.clone(),
            "text": page.distilled_text.clone(),
            "links": links.clone()
        });
        let outcome = extract_structured(&page, candidate, schema.clone());
        Ok(PageExtract {
            url: page.url,
            title: page.title,
            text: page.distilled_text,
            links,
            schema,
            valid: outcome.valid,
            errors: outcome.errors,
        })
    }

    fn replace_current(&mut self, page: PageState) {
        let mut page = page;
        if page.active_tab_id.is_none() {
            page.active_tab_id = self.tabs.active().map(|tab| tab.id.clone());
        }
        self.store_active_tab_page(&page);
        if let Some(slot) = self.history.get_mut(self.history_index) {
            *slot = page;
        }
    }

    fn stamp_active_tab(&mut self, page: &mut PageState) {
        if self.tabs.is_empty() {
            self.tabs.open(page.url.clone());
        }
        if let Some(tab_id) = self
            .tabs
            .update_active(page.url.clone(), page.title.clone())
        {
            page.active_tab_id = Some(tab_id);
        }
    }

    fn store_active_tab_page(&mut self, page: &PageState) {
        if let Some(tab_id) = page
            .active_tab_id
            .clone()
            .or_else(|| self.tabs.active().map(|tab| tab.id.clone()))
        {
            self.tab_pages.insert(tab_id, page.clone());
        }
    }

    fn apply_select(
        &mut self,
        element_id: &str,
        value: &str,
        domain: &str,
        policy: &BrowserActionPolicy,
        receipt: &mut Value,
    ) -> BrowserEngineResult<PageState> {
        let mut page = self.observe()?;
        let element = page
            .interactive_elements
            .iter_mut()
            .find(|element| element.element_id == element_id)
            .ok_or_else(|| BrowserEngineError::ElementNotFound {
                element_id: element_id.to_string(),
            })?;
        let resolved = policy.sensitive_data.resolve_placeholders(domain, value);
        let masked = policy.sensitive_data.mask(domain, &resolved);
        element.value = Some(masked.masked.clone());
        *receipt = json!({
            "engine": "fetch_cascade_browser_engine",
            "executor": "local_select_update",
            "masked_value": masked.masked,
            "sensitive_keys": masked.used_keys
        });
        self.replace_current(page.clone());
        Ok(page)
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
        valid: true,
        errors: Vec::new(),
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

fn enforce_domain_policy(
    action: &BrowserAction,
    page: &PageState,
    policy: &BrowserActionPolicy,
) -> BrowserEngineResult<()> {
    let Some(target) = action_target_url(action, page) else {
        return Ok(());
    };
    match DomainPolicy::new(policy.permitted_domains.clone()).evaluate(&target) {
        NavigationDecision::Permitted => Ok(()),
        NavigationDecision::Refused { reason } => Err(BrowserEngineError::ActionBlocked { reason }),
    }
}

fn action_target_url(action: &BrowserAction, page: &PageState) -> Option<String> {
    match action {
        BrowserAction::OpenTab { url } => Some(url.clone()),
        BrowserAction::Click { element_id } => page
            .interactive_elements
            .iter()
            .find(|element| element.element_id == *element_id && element.role == "link")
            .and_then(|element| element.value.clone()),
        _ => None,
    }
}

fn domain_for_sensitive_data(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "*".to_string())
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
        active_tab_id: None,
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
                        degraded: false,
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
                        degraded: false,
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
                        degraded: false,
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
                        degraded: false,
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
                        degraded: false,
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
            ..BrowserActionPolicy::default()
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

    #[tokio::test]
    async fn type_masks_sensitive_placeholders_in_returned_page_and_receipt() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        let page = page_state_from_html(
            "https://example.com/",
            r#"<html><input name="password" value="" /></html>"#,
        )
        .expect("page");
        engine.history.push(page);

        let mut sensitive_data = SensitiveData::new();
        sensitive_data.set("example.com", "password", "hunter2");
        let policy = BrowserActionPolicy {
            sensitive_data,
            ..BrowserActionPolicy::default()
        };
        let outcome = engine
            .act(
                BrowserAction::Type {
                    element_id: "e0".to_string(),
                    text: "{{secret:password}}".to_string(),
                },
                &policy,
            )
            .await
            .expect("type");

        let page_value = outcome.page.interactive_elements[0]
            .value
            .as_deref()
            .unwrap();
        assert!(!page_value.contains("hunter2"));
        assert!(page_value.contains("<secret:example.com/password>"));
        let receipt = outcome.receipt.to_string();
        assert!(!receipt.contains("hunter2"));
        assert!(receipt.contains("password"));
    }

    #[tokio::test]
    async fn upload_file_requires_allowlisted_root() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        let page = page_state_from_html(
            "https://example.com/",
            r#"<html><input type="file" name="avatar" /></html>"#,
        )
        .expect("page");
        engine.history.push(page);

        let err = engine
            .act(
                BrowserAction::UploadFile {
                    element_id: "e0".to_string(),
                    path: "/tmp/avatar.png".to_string(),
                },
                &BrowserActionPolicy::default(),
            )
            .await
            .expect_err("upload should be blocked without roots");
        assert!(matches!(err, BrowserEngineError::ActionBlocked { .. }));

        let outcome = engine
            .act(
                BrowserAction::UploadFile {
                    element_id: "e0".to_string(),
                    path: "/tmp/avatar.png".to_string(),
                },
                &BrowserActionPolicy {
                    upload_roots: vec!["/tmp".to_string()],
                    ..BrowserActionPolicy::default()
                },
            )
            .await
            .expect("allowlisted upload");
        assert_eq!(
            outcome.page.interactive_elements[0].value.as_deref(),
            Some("/tmp/avatar.png")
        );
    }

    #[tokio::test]
    async fn domain_policy_blocks_off_domain_link_before_fetch() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        let page = page_state_from_html(
            "https://example.com/",
            r#"<html><a href="https://outside.test/">Outside</a></html>"#,
        )
        .expect("page");
        engine.history.push(page);

        let err = engine
            .act(
                BrowserAction::Click {
                    element_id: "e0".to_string(),
                },
                &BrowserActionPolicy {
                    permitted_domains: vec!["example.com".to_string()],
                    ..BrowserActionPolicy::default()
                },
            )
            .await
            .expect_err("off-domain link should be blocked before navigation");
        assert!(matches!(err, BrowserEngineError::ActionBlocked { .. }));
    }

    #[tokio::test]
    async fn switch_tab_returns_page_with_active_tab_id() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        let first_tab = engine.tabs.open("https://example.com/one");
        let second_tab = engine.tabs.open("https://example.com/two");
        let mut first_page = page_state_from_html("https://example.com/one", "<title>One</title>")
            .expect("first page");
        first_page.active_tab_id = Some(first_tab.clone());
        let mut second_page = page_state_from_html("https://example.com/two", "<title>Two</title>")
            .expect("second page");
        second_page.active_tab_id = Some(second_tab.clone());
        engine
            .tab_pages
            .insert(first_tab.clone(), first_page.clone());
        engine.tab_pages.insert(second_tab.clone(), second_page);
        engine.history.push(first_page);

        let outcome = engine
            .act(
                BrowserAction::SwitchTab {
                    tab_id: second_tab.clone(),
                },
                &BrowserActionPolicy::default(),
            )
            .await
            .expect("switch tab");
        assert_eq!(
            outcome.page.active_tab_id.as_deref(),
            Some(second_tab.as_str())
        );
        assert_eq!(outcome.page.title, "Two");
    }

    #[test]
    fn extract_reports_schema_validation_errors() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        let page =
            page_state_from_html("https://example.com/", "<title>Docs</title>").expect("page");
        engine.history.push(page);

        let extract = engine
            .extract(json!({
                "type": "object",
                "required": ["items"],
                "properties": {
                    "items": { "type": "array" }
                }
            }))
            .expect("extract");
        assert!(!extract.valid);
        assert!(extract.errors.iter().any(|error| error.contains("$.items")));
    }

    #[test]
    fn web_consume_request_defaults_respect_robots_true() {
        let request: WebConsumeRequest =
            serde_json::from_value(json!({ "run_id": "r1", "url": "https://example.com/" }))
                .expect("deserialize");
        assert!(request.respect_robots);
    }
}
