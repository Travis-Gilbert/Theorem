//! Playwright-shaped automation core for the substrate browser.
//!
//! This module is Servo-free by design. It defines the locator, actionability,
//! context, routing, assertion, and receipt contracts that a live Servo embedder
//! can satisfy with `evaluate_javascript` and input synthesis, while the
//! existing fetch-cascade engine can exercise the same API in fast unit tests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::browser_engine::{
    BrowserAction, BrowserActionOutcome, BrowserActionPolicy, BrowserEngineError,
    BrowserEngineResult, ElementBox, FetchCascadeBrowserEngine, InteractiveElement, PageState,
};

pub const PLAYWRIGHT_SELECTOR_UPSTREAM: &str =
    "microsoft/playwright v1.61.0 tag 1cc5a90cfa3eaa430b1a991963100f95126caa47";
pub const PLAYWRIGHT_SELECTOR_LICENSE: &str = "Apache-2.0";
pub const SELECTOR_BRIDGE_SCRIPT: &str = include_str!("vendor/playwright_selector_bridge.js");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SelectorEngineProvenance {
    pub upstream: String,
    pub license: String,
    pub source_paths: Vec<String>,
    pub bridge: String,
}

pub fn selector_engine_provenance() -> SelectorEngineProvenance {
    SelectorEngineProvenance {
        upstream: PLAYWRIGHT_SELECTOR_UPSTREAM.to_string(),
        license: PLAYWRIGHT_SELECTOR_LICENSE.to_string(),
        source_paths: vec![
            "packages/injected/src/injectedScript.ts".to_string(),
            "packages/injected/src/selectorEngine.ts".to_string(),
            "packages/injected/src/selectorEvaluator.ts".to_string(),
            "packages/injected/src/roleSelectorEngine.ts".to_string(),
            "packages/isomorphic/selectorParser.ts".to_string(),
        ],
        bridge: "rustyred-web/src/vendor/playwright_selector_bridge.js".to_string(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Locator {
    page: Option<String>,
    steps: Vec<LocatorStep>,
}

impl Locator {
    pub fn css(selector: impl Into<String>) -> Self {
        Self {
            page: None,
            steps: vec![LocatorStep::Css {
                selector: selector.into(),
            }],
        }
    }

    pub fn get_by_role(role: impl Into<String>, opts: RoleOptions) -> Self {
        Self {
            page: None,
            steps: vec![LocatorStep::Role {
                role: role.into(),
                name: opts.name,
            }],
        }
    }

    pub fn get_by_text(text: impl Into<String>, exact: bool) -> Self {
        Self {
            page: None,
            steps: vec![LocatorStep::Text {
                text: text.into(),
                exact,
            }],
        }
    }

    pub fn get_by_label(text: impl Into<String>) -> Self {
        Self {
            page: None,
            steps: vec![LocatorStep::Label { text: text.into() }],
        }
    }

    pub fn get_by_test_id(id: impl Into<String>) -> Self {
        Self {
            page: None,
            steps: vec![LocatorStep::TestId { id: id.into() }],
        }
    }

    pub fn frame(mut self, page: impl Into<String>) -> Self {
        self.page = Some(page.into());
        self
    }

    pub fn filter(mut self, has: Option<Locator>, has_text: Option<impl Into<String>>) -> Self {
        self.steps.push(LocatorStep::Filter {
            has: has.map(Box::new),
            has_text: has_text.map(Into::into),
        });
        self
    }

    pub fn nth(mut self, index: usize) -> Self {
        self.steps.push(LocatorStep::Nth(index));
        self
    }

    pub fn selector_summary(&self) -> String {
        self.steps
            .iter()
            .map(LocatorStep::summary)
            .collect::<Vec<_>>()
            .join(" >> ")
    }

    pub fn resolve(&self, page: &PageState) -> Vec<ElementHandle> {
        let mut current: Vec<InteractiveElement> = page.interactive_elements.clone();
        for step in &self.steps {
            current = apply_locator_step(current, page, step);
        }
        current.into_iter().map(ElementHandle::from).collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LocatorStep {
    Css {
        selector: String,
    },
    Role {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Text {
        text: String,
        exact: bool,
    },
    Label {
        text: String,
    },
    TestId {
        id: String,
    },
    Filter {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        has: Option<Box<Locator>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        has_text: Option<String>,
    },
    Nth(usize),
}

impl LocatorStep {
    fn summary(&self) -> String {
        match self {
            LocatorStep::Css { selector } => format!("css={selector}"),
            LocatorStep::Role { role, name } => match name {
                Some(name) => format!("role={role}[name={name}]"),
                None => format!("role={role}"),
            },
            LocatorStep::Text { text, exact } => {
                format!("text={text}{}", if *exact { "[exact]" } else { "" })
            }
            LocatorStep::Label { text } => format!("label={text}"),
            LocatorStep::TestId { id } => format!("test_id={id}"),
            LocatorStep::Filter { has_text, .. } => match has_text {
                Some(text) => format!("filter[has_text={text}]"),
                None => "filter".to_string(),
            },
            LocatorStep::Nth(index) => format!("nth={index}"),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoleOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementHandle {
    pub handle: String,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<ElementBox>,
    pub visible: bool,
    pub enabled: bool,
    pub editable: bool,
    pub degraded: bool,
}

impl From<InteractiveElement> for ElementHandle {
    fn from(element: InteractiveElement) -> Self {
        Self {
            handle: element.element_id,
            role: element.role,
            name: element.name,
            value: element.value,
            test_id: element.test_id,
            rect: element.bbox,
            visible: element.visible,
            enabled: element.enabled,
            editable: element.editable,
            degraded: element.degraded,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Actionability {
    pub attached: bool,
    pub visible: bool,
    pub stable: bool,
    pub enabled: bool,
    pub editable: bool,
    pub receives_events: bool,
}

impl Actionability {
    pub fn from_handle(handle: &ElementHandle) -> Self {
        let has_area = handle.rect.as_ref().map(has_area).unwrap_or(true);
        let visible = handle.visible && has_area;
        Self {
            attached: true,
            visible,
            stable: true,
            enabled: handle.enabled,
            editable: handle.editable && handle.enabled,
            receives_events: visible && handle.enabled && !handle.degraded,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionabilityCheck {
    Attached,
    Visible,
    Stable,
    Enabled,
    Editable,
    ReceivesEvents,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionabilityRequirement {
    pub checks: Vec<ActionabilityCheck>,
}

impl ActionabilityRequirement {
    pub fn for_action(action: &LocatorAction, force: bool) -> Self {
        let mut checks = match action {
            LocatorAction::Click
            | LocatorAction::DoubleClick
            | LocatorAction::Check
            | LocatorAction::SetChecked { .. }
            | LocatorAction::Tap => vec![
                ActionabilityCheck::Attached,
                ActionabilityCheck::Visible,
                ActionabilityCheck::Stable,
                ActionabilityCheck::ReceivesEvents,
                ActionabilityCheck::Enabled,
            ],
            LocatorAction::Fill { .. } => vec![
                ActionabilityCheck::Attached,
                ActionabilityCheck::Visible,
                ActionabilityCheck::Enabled,
                ActionabilityCheck::Editable,
            ],
            LocatorAction::Hover | LocatorAction::ScrollIntoView => vec![
                ActionabilityCheck::Attached,
                ActionabilityCheck::Visible,
                ActionabilityCheck::Stable,
                ActionabilityCheck::ReceivesEvents,
            ],
            LocatorAction::SelectOption { .. } | LocatorAction::SetInputFiles { .. } => vec![
                ActionabilityCheck::Attached,
                ActionabilityCheck::Visible,
                ActionabilityCheck::Enabled,
            ],
        };
        if force {
            checks.retain(|check| *check != ActionabilityCheck::ReceivesEvents);
        }
        Self { checks }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionabilityVerdict {
    pub passed: bool,
    pub attempts: usize,
    pub checks: Actionability,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<ActionabilityCheck>,
}

impl ActionabilityVerdict {
    pub fn evaluate(handle: &ElementHandle, requirement: &ActionabilityRequirement) -> Self {
        let checks = Actionability::from_handle(handle);
        let missing = requirement
            .checks
            .iter()
            .copied()
            .filter(|check| !check_passed(&checks, *check))
            .collect::<Vec<_>>();
        Self {
            passed: missing.is_empty(),
            attempts: 1,
            checks,
            missing,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionOptions {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub force: bool,
}

impl Default for ActionOptions {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
            force: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LocatorAction {
    Click,
    DoubleClick,
    Check,
    SetChecked { checked: bool },
    Tap,
    Fill { value: String },
    Hover,
    ScrollIntoView,
    SelectOption { value: String },
    SetInputFiles { paths: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationActionReceipt {
    pub action: LocatorAction,
    pub selector: String,
    pub handle: ElementHandle,
    pub actionability: ActionabilityVerdict,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_action: Option<BrowserAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_receipt: Option<Value>,
}

pub async fn perform_locator_action(
    engine: &mut FetchCascadeBrowserEngine,
    locator: &Locator,
    action: LocatorAction,
    options: ActionOptions,
    policy: &BrowserActionPolicy,
) -> BrowserEngineResult<AutomationActionReceipt> {
    let page = engine.observe()?;
    let Some(handle) = locator.resolve(&page).into_iter().next() else {
        return Err(BrowserEngineError::ElementNotFound {
            element_id: locator.selector_summary(),
        });
    };
    let requirement = ActionabilityRequirement::for_action(&action, options.force);
    let actionability = ActionabilityVerdict::evaluate(&handle, &requirement);
    if !actionability.passed {
        return Ok(AutomationActionReceipt {
            action,
            selector: locator.selector_summary(),
            handle,
            actionability,
            applied: false,
            browser_action: None,
            engine_receipt: None,
        });
    }
    let browser_action = browser_action_for_locator_action(&handle, &action)?;
    let BrowserActionOutcome {
        applied, receipt, ..
    } = engine.act(browser_action.clone(), policy).await?;
    Ok(AutomationActionReceipt {
        action,
        selector: locator.selector_summary(),
        handle,
        actionability,
        applied,
        browser_action: Some(browser_action),
        engine_receipt: Some(receipt),
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextOptions {
    pub context_id: String,
    #[serde(default)]
    pub storage_partition: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            context_id: "context:default".to_string(),
            storage_partition: "default".to_string(),
            permissions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Context {
    pub options: ContextOptions,
    #[serde(default)]
    pub routes: Vec<RouteRule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cookies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub storage: BTreeMap<String, String>,
}

impl Context {
    pub fn new(options: ContextOptions) -> Self {
        Self {
            options,
            routes: Vec::new(),
            cookies: BTreeMap::new(),
            storage: BTreeMap::new(),
        }
    }

    pub fn route(&mut self, pattern: UrlPattern, action: RouteAction) {
        self.routes.push(RouteRule { pattern, action });
    }

    pub fn route_for(&self, url: &str) -> Option<RouteDecision> {
        self.routes
            .iter()
            .find(|rule| rule.pattern.matches(url))
            .map(|rule| RouteDecision {
                url: url.to_string(),
                action: rule.action.clone(),
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteRule {
    pub pattern: UrlPattern,
    pub action: RouteAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum UrlPattern {
    Any,
    Exact { url: String },
    Prefix { prefix: String },
    Contains { needle: String },
}

impl UrlPattern {
    pub fn matches(&self, url: &str) -> bool {
        match self {
            UrlPattern::Any => true,
            UrlPattern::Exact { url: expected } => url == expected,
            UrlPattern::Prefix { prefix } => url.starts_with(prefix),
            UrlPattern::Contains { needle } => url.contains(needle),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RouteAction {
    Continue,
    Abort,
    Fulfill {
        status: u16,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    },
    ContinueWith {
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub url: String,
    pub action: RouteAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AssertionKind {
    Visible,
    Enabled,
    Text { expected: String, exact: bool },
    Count { expected: usize },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssertionResult {
    pub passed: bool,
    pub attempts: usize,
    pub actual_count: usize,
    pub message: String,
}

pub struct LocatorExpectation {
    locator: Locator,
}

pub fn expect(locator: Locator) -> LocatorExpectation {
    LocatorExpectation { locator }
}

impl LocatorExpectation {
    pub fn to_be_visible(&self, page: &PageState) -> AssertionResult {
        self.evaluate(page, AssertionKind::Visible)
    }

    pub fn to_be_enabled(&self, page: &PageState) -> AssertionResult {
        self.evaluate(page, AssertionKind::Enabled)
    }

    pub fn to_have_text(&self, page: &PageState, expected: impl Into<String>) -> AssertionResult {
        self.evaluate(
            page,
            AssertionKind::Text {
                expected: expected.into(),
                exact: false,
            },
        )
    }

    pub fn to_have_count(&self, page: &PageState, expected: usize) -> AssertionResult {
        self.evaluate(page, AssertionKind::Count { expected })
    }

    pub fn evaluate(&self, page: &PageState, assertion: AssertionKind) -> AssertionResult {
        let handles = self.locator.resolve(page);
        let passed = match &assertion {
            AssertionKind::Visible => handles
                .first()
                .map(|handle| Actionability::from_handle(handle).visible)
                .unwrap_or(false),
            AssertionKind::Enabled => handles
                .first()
                .map(|handle| handle.enabled)
                .unwrap_or(false),
            AssertionKind::Text { expected, exact } => handles.iter().any(|handle| {
                text_matches(&handle.name, expected, *exact)
                    || handle
                        .value
                        .as_deref()
                        .map(|value| text_matches(value, expected, *exact))
                        .unwrap_or(false)
            }),
            AssertionKind::Count { expected } => handles.len() == *expected,
        };
        AssertionResult {
            passed,
            attempts: 1,
            actual_count: handles.len(),
            message: assertion_message(&assertion, passed, handles.len()),
        }
    }
}

fn apply_locator_step(
    elements: Vec<InteractiveElement>,
    page: &PageState,
    step: &LocatorStep,
) -> Vec<InteractiveElement> {
    match step {
        LocatorStep::Css { selector } => elements
            .into_iter()
            .filter(|element| css_selector_matches(element, selector))
            .collect(),
        LocatorStep::Role { role, name } => elements
            .into_iter()
            .filter(|element| element.role.eq_ignore_ascii_case(role))
            .filter(|element| {
                name.as_ref()
                    .map(|name| text_matches(&element.name, name, false))
                    .unwrap_or(true)
            })
            .collect(),
        LocatorStep::Text { text, exact } => elements
            .into_iter()
            .filter(|element| {
                text_matches(&element.name, text, *exact)
                    || element
                        .value
                        .as_deref()
                        .map(|value| text_matches(value, text, *exact))
                        .unwrap_or(false)
            })
            .collect(),
        LocatorStep::Label { text } => elements
            .into_iter()
            .filter(|element| text_matches(&element.name, text, false))
            .collect(),
        LocatorStep::TestId { id } => elements
            .into_iter()
            .filter(|element| {
                element.test_id.as_deref() == Some(id.as_str())
                    || element.element_id == *id
                    || element.name == *id
            })
            .collect(),
        LocatorStep::Filter { has, has_text } => elements
            .into_iter()
            .filter(|element| {
                has_text
                    .as_ref()
                    .map(|text| {
                        text_matches(&element.name, text, false)
                            || element
                                .value
                                .as_deref()
                                .map(|value| text_matches(value, text, false))
                                .unwrap_or(false)
                    })
                    .unwrap_or(true)
            })
            .filter(|_| {
                has.as_ref()
                    .map(|locator| !locator.resolve(page).is_empty())
                    .unwrap_or(true)
            })
            .collect(),
        LocatorStep::Nth(index) => elements.into_iter().nth(*index).into_iter().collect(),
    }
}

fn browser_action_for_locator_action(
    handle: &ElementHandle,
    action: &LocatorAction,
) -> BrowserEngineResult<BrowserAction> {
    let element_id = handle.handle.clone();
    match action {
        LocatorAction::Click
        | LocatorAction::DoubleClick
        | LocatorAction::Check
        | LocatorAction::SetChecked { .. }
        | LocatorAction::Tap => Ok(BrowserAction::Click { element_id }),
        LocatorAction::Fill { value } => Ok(BrowserAction::Type {
            element_id,
            text: value.clone(),
        }),
        LocatorAction::Hover => Ok(BrowserAction::Hover { element_id }),
        LocatorAction::ScrollIntoView => Ok(BrowserAction::ScrollToElement { element_id }),
        LocatorAction::SelectOption { value } => Ok(BrowserAction::SelectOption {
            element_id,
            value: value.clone(),
        }),
        LocatorAction::SetInputFiles { paths } => {
            let Some(path) = paths.first() else {
                return Err(BrowserEngineError::UnsupportedAction {
                    reason: "set_input_files requires at least one path".to_string(),
                });
            };
            Ok(BrowserAction::UploadFile {
                element_id,
                path: path.clone(),
            })
        }
    }
}

fn css_selector_matches(element: &InteractiveElement, selector: &str) -> bool {
    if selector == "*" {
        return true;
    }
    if let Some(id) = selector.strip_prefix('#') {
        return element.element_id == id
            || element.name == id
            || element.test_id.as_deref() == Some(id);
    }
    if let Some(test_id) = selector
        .strip_prefix("[data-testid=\"")
        .and_then(|tail| tail.strip_suffix("\"]"))
    {
        return element.test_id.as_deref() == Some(test_id);
    }
    element.role.eq_ignore_ascii_case(selector)
}

fn text_matches(actual: &str, expected: &str, exact: bool) -> bool {
    if exact {
        actual == expected
    } else {
        actual
            .to_ascii_lowercase()
            .contains(&expected.to_ascii_lowercase())
    }
}

fn check_passed(checks: &Actionability, check: ActionabilityCheck) -> bool {
    match check {
        ActionabilityCheck::Attached => checks.attached,
        ActionabilityCheck::Visible => checks.visible,
        ActionabilityCheck::Stable => checks.stable,
        ActionabilityCheck::Enabled => checks.enabled,
        ActionabilityCheck::Editable => checks.editable,
        ActionabilityCheck::ReceivesEvents => checks.receives_events,
    }
}

fn has_area(rect: &ElementBox) -> bool {
    rect.width > 0 && rect.height > 0
}

fn assertion_message(assertion: &AssertionKind, passed: bool, actual_count: usize) -> String {
    let status = if passed { "passed" } else { "failed" };
    format!("{status}: {assertion:?}; actual_count={actual_count}")
}

fn default_timeout_ms() -> u64 {
    30_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_engine::page_state_from_html;
    use crate::{FetchCascade, FetchCascadeOptions};

    fn page() -> PageState {
        page_state_from_html(
            "https://example.com/",
            r#"
            <html>
              <button data-testid="save-button" aria-label="Save">Save</button>
              <input name="q" value="" />
              <input name="locked" value="" disabled />
              <a href="/docs" aria-label="Docs">Docs</a>
            </html>
            "#,
        )
        .expect("page")
    }

    #[test]
    fn locators_resolve_role_text_and_test_id() {
        let page = page();
        let role = Locator::get_by_role(
            "button",
            RoleOptions {
                name: Some("Save".to_string()),
            },
        );
        assert_eq!(
            role.resolve(&page)[0].test_id.as_deref(),
            Some("save-button")
        );

        let text = Locator::get_by_text("Docs", true);
        assert_eq!(text.resolve(&page)[0].role, "link");

        let test_id = Locator::get_by_test_id("save-button");
        assert_eq!(test_id.resolve(&page)[0].name, "Save");
    }

    #[test]
    fn actionability_blocks_disabled_fill_without_applying_action() {
        let page = page();
        let locator = Locator::get_by_label("locked");
        let handle = locator.resolve(&page).into_iter().next().expect("handle");
        let verdict = ActionabilityVerdict::evaluate(
            &handle,
            &ActionabilityRequirement::for_action(
                &LocatorAction::Fill {
                    value: "nope".to_string(),
                },
                false,
            ),
        );
        assert!(!verdict.passed);
        assert!(verdict.missing.contains(&ActionabilityCheck::Enabled));
    }

    #[test]
    fn force_drops_receives_events_but_keeps_visible_and_enabled() {
        let handle = ElementHandle {
            handle: "e0".to_string(),
            role: "button".to_string(),
            name: "Save".to_string(),
            value: None,
            test_id: None,
            rect: None,
            visible: true,
            enabled: true,
            editable: false,
            degraded: true,
        };
        let strict = ActionabilityVerdict::evaluate(
            &handle,
            &ActionabilityRequirement::for_action(&LocatorAction::Click, false),
        );
        assert!(!strict.passed);
        assert!(strict.missing.contains(&ActionabilityCheck::ReceivesEvents));

        let forced = ActionabilityVerdict::evaluate(
            &handle,
            &ActionabilityRequirement::for_action(&LocatorAction::Click, true),
        );
        assert!(forced.passed);
    }

    #[tokio::test]
    async fn locator_action_records_actionability_and_engine_receipt() {
        let cascade = FetchCascade::new(FetchCascadeOptions::http2_only(
            "RustyWeb test".to_string(),
            5,
        ))
        .expect("cascade");
        let mut engine = FetchCascadeBrowserEngine::new(cascade, 1024);
        engine.seed_page_state(page());

        let receipt = perform_locator_action(
            &mut engine,
            &Locator::get_by_label("q"),
            LocatorAction::Fill {
                value: "servo".to_string(),
            },
            ActionOptions::default(),
            &BrowserActionPolicy::default(),
        )
        .await
        .expect("fill");

        assert!(receipt.applied);
        assert!(receipt.actionability.passed);
        assert!(matches!(
            receipt.browser_action,
            Some(BrowserAction::Type { .. })
        ));
        assert!(receipt.engine_receipt.is_some());
        assert_eq!(
            engine.observe().unwrap().interactive_elements[1]
                .value
                .as_deref(),
            Some("servo")
        );
    }

    #[test]
    fn context_routes_and_storage_are_isolated() {
        let mut first = Context::new(ContextOptions {
            context_id: "ctx:first".to_string(),
            storage_partition: "first".to_string(),
            permissions: vec!["geolocation".to_string()],
        });
        first
            .cookies
            .insert("session".to_string(), "one".to_string());
        first.route(
            UrlPattern::Contains {
                needle: ".png".to_string(),
            },
            RouteAction::Abort,
        );

        let second = Context::new(ContextOptions {
            context_id: "ctx:second".to_string(),
            storage_partition: "second".to_string(),
            permissions: Vec::new(),
        });

        assert!(first.route_for("https://example.com/logo.png").is_some());
        assert!(second.route_for("https://example.com/logo.png").is_none());
        assert!(second.cookies.get("session").is_none());
    }

    #[test]
    fn expectations_report_web_first_predicates() {
        let page = page();
        assert!(
            expect(Locator::get_by_test_id("save-button"))
                .to_be_visible(&page)
                .passed
        );
        assert!(
            expect(Locator::get_by_role("text", RoleOptions::default()))
                .to_have_count(&page, 2)
                .passed
        );
        assert!(
            !expect(Locator::get_by_label("missing"))
                .to_be_visible(&page)
                .passed
        );
    }

    #[test]
    fn selector_provenance_keeps_upstream_license_visible() {
        let provenance = selector_engine_provenance();
        assert_eq!(provenance.license, "Apache-2.0");
        assert!(provenance.upstream.contains("playwright"));
        assert!(SELECTOR_BRIDGE_SCRIPT.contains("theoremQuerySelectorAll"));
    }
}
