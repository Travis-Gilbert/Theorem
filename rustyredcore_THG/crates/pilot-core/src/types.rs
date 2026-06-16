//! Pure, serde-only data types for the automation core: element geometry
//! ([`ElementBox`]), the actionable element ([`InteractiveElement`]), the action
//! vocabulary ([`BrowserAction`] / [`WaitCondition`]).
//!
//! Migrated in place from rustyred-web's `browser_engine.rs`; rustyred-web
//! re-exports these so its consumers are unchanged. Zero substrate, zero Servo.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::masking::SensitiveData;

/// A rectangle in CSS pixels (viewport-relative), as returned by
/// `getBoundingClientRect`. The driver converts a rect center to a device point
/// before coordinate-synthesis actuation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ElementBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// One actionable element in a page snapshot. `element_id` is the stable handle
/// (the geometry-snapshot `data-theorem-id` stamp, id-space Option A).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InteractiveElement {
    pub element_id: String,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<ElementBox>,
    pub visible: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub editable: bool,
    /// job-007 D5: the engine has not yet rolled a proper interactive role or
    /// bounds for this node, so it is surfaced degraded. A degraded element is
    /// still operable (keyboard fallback), but the driving model should prefer
    /// keyboard or vision over a precise click. Additive; serde-defaults false
    /// so older receipts and the HTML reader path round-trip unchanged.
    #[serde(default)]
    pub degraded: bool,
}

/// A page snapshot: the actionable elements plus minimal page metadata. The
/// `fetch` field is an opaque, backend-specific transport summary (rustyred-web
/// stores its fetch-cascade summary there; the live Servo driver leaves it
/// `None`), keeping the core free of any fetch/transport type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub distilled_text: String,
    pub interactive_elements: Vec<InteractiveElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch: Option<Value>,
}

/// A wait predicate for the action vocabulary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitCondition {
    LoadComplete,
    ElementVisible(String),
    Millis(u64),
}

/// The low-level action vocabulary a backend executes. The Playwright-shaped
/// `Locator` API resolves to these.
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
    Hover {
        element_id: String,
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

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The result type for engine/driver operations.
pub type BrowserEngineResult<T> = Result<T, BrowserEngineError>;

/// Errors a backend driver can return.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BrowserEngineError {
    NoCurrentPage,
    ElementNotFound { element_id: String },
    UnsupportedAction { reason: String },
    ActionBlocked { reason: String },
    /// A backend-specific error (e.g. a fetch/transport failure in the
    /// fetch-cascade engine). The driver crate maps its own error type here.
    Backend { message: String },
}

/// Governance policy for an actuation: what is permitted, plus the
/// sensitive-data secrets resolved and masked at the engine boundary.
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
