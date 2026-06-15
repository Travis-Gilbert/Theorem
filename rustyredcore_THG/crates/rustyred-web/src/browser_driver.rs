//! The Servo-targeting driver seam for the Playwright-class automation core.
//!
//! `rustyred-web` is Servo-free by design. [`browser_automation`](crate::browser_automation)
//! defines the Playwright-shaped contract (locator, actionability, assertions,
//! receipts). This module adds the seam that lets that one contract run against
//! two backends without the contract knowing which:
//!
//! * [`FetchCascadeBrowserEngine`] - fast, local, HTML-parse snapshot. The unit
//!   test path; no libservo, builds in seconds.
//! * a live `servo::WebView` - an `evaluate_javascript` geometry snapshot plus
//!   `notify_input_event` synthesis, implemented in `apps/browser` (CI-only).
//!
//! The driver-generic [`run_action`] adds the auto-wait that the one-shot
//! [`perform_locator_action`](crate::browser_automation::perform_locator_action)
//! lacks: it re-snapshots on a cadence until the actionability gate passes or the
//! deadline elapses, then actuates by the E5/E6 coordinate transform. The
//! actionable element id is the geometry-snapshot `data-theorem-id` stamp
//! (Travis-approved id-space Option A); the AccessKit `NodeId` stays in the
//! structural overlay only.
//!
//! Actuation is an enum ([`ActuationKind`]) so the Travis-approved Servo issue
//! #4344 route ([`ActuationKind::SemanticActivation`]) is a first-class variant
//! now, landed in the engine phase by the Servo fork rather than bolted on later.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration, Instant};

use crate::browser_automation::{
    ActionabilityRequirement, ActionabilityVerdict, ActionOptions, AutomationActionReceipt,
    ElementHandle, Locator, LocatorAction,
};
use crate::browser_engine::{
    BrowserAction, BrowserActionOutcome, BrowserActionPolicy, BrowserEngineError,
    BrowserEngineResult, ElementBox, FetchCascadeBrowserEngine, InteractiveElement, PageState,
};

/// One re-snapshot per ~16ms, i.e. roughly one display frame. Playwright's
/// actionability poll cadence; the live driver can later replace this with the
/// engine layout-settle signal (job-008 D2) so the poll is event-driven.
const SNAPSHOT_POLL_MS: u64 = 16;

// ---------------------------------------------------------------------------
// Coordinate space (E5/E6).
// ---------------------------------------------------------------------------

/// A point in the rendering context's device-pixel space, which is what
/// `WebView::notify_input_event` consumes. `getBoundingClientRect` is CSS pixels
/// viewport-relative, so a click target is converted through
/// [`device_point_at_rect_center`] before actuation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DevicePoint {
    pub x: f32,
    pub y: f32,
}

impl DevicePoint {
    pub const ZERO: DevicePoint = DevicePoint { x: 0.0, y: 0.0 };
}

/// The CSS-pixel center of a rectangle.
pub fn rect_center_css(rect: &ElementBox) -> (f32, f32) {
    (
        rect.x as f32 + rect.width as f32 / 2.0,
        rect.y as f32 + rect.height as f32 / 2.0,
    )
}

/// E6: `device_point = css_point * device_pixels_per_css_pixel + webview_origin`.
/// Getting this wrong lands a synthetic click on the wrong element silently, so
/// it is a first-class, unit-tested transform rather than an inline expression.
pub fn css_to_device_point(css_x: f32, css_y: f32, dppx: f32, origin: DevicePoint) -> DevicePoint {
    DevicePoint {
        x: css_x * dppx + origin.x,
        y: css_y * dppx + origin.y,
    }
}

/// The device point at a rectangle's center, the click/hover target for
/// coordinate synthesis.
pub fn device_point_at_rect_center(rect: &ElementBox, dppx: f32, origin: DevicePoint) -> DevicePoint {
    let (cx, cy) = rect_center_css(rect);
    css_to_device_point(cx, cy, dppx, origin)
}

// ---------------------------------------------------------------------------
// The geometry snapshot (E4) + its Servo-free parser, realizing id-space Option A.
// ---------------------------------------------------------------------------

/// E4: the injected script the live driver runs through `evaluate_javascript`.
/// It stamps a deterministic `data-theorem-id` (document order) on each
/// actionable element - that stamp is the actionable handle (Option A) - and
/// returns a JSON array the driver feeds to [`page_state_from_snapshot_json`].
/// It mirrors the role/name semantics of the vendored selector bridge
/// ([`SELECTOR_BRIDGE_SCRIPT`](crate::browser_automation::SELECTOR_BRIDGE_SCRIPT)),
/// keeping one source of truth for how an element's role and accessible name are
/// computed. The output shape is the contract; see the parser below.
pub const GEOMETRY_SNAPSHOT_SCRIPT: &str = r#"
(function () {
  var EDITABLE_TYPES = ["text","search","email","password","url","tel","number","date","datetime-local","month","time","week"];
  function roleOf(el) {
    var explicit = el.getAttribute("role");
    if (explicit) return explicit.toLowerCase();
    var tag = el.tagName.toLowerCase();
    if (tag === "a" && el.hasAttribute("href")) return "link";
    if (tag === "button") return "button";
    if (tag === "textarea") return "textbox";
    if (tag === "select") return "select";
    if (tag === "input") return (el.getAttribute("type") || "text").toLowerCase();
    return tag;
  }
  function textOf(el) {
    return [
      el.getAttribute("aria-label"),
      el.getAttribute("title"),
      el.getAttribute("placeholder"),
      el.getAttribute("alt"),
      el.textContent
    ].filter(Boolean).join(" ").trim();
  }
  function visibleOf(el, rect) {
    if (!rect || rect.width <= 0 || rect.height <= 0) return false;
    var style = window.getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden") return false;
    if (el.offsetParent === null && style.position !== "fixed") return false;
    return true;
  }
  var nodes = document.querySelectorAll("a[href],button,input,select,textarea,[role],[data-testid]");
  var out = [];
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    var handle = "t" + i;
    el.setAttribute("data-theorem-id", handle);
    var r = el.getBoundingClientRect();
    var tag = el.tagName.toLowerCase();
    var type = tag === "input" ? (el.getAttribute("type") || "text").toLowerCase() : "";
    var disabled = el.disabled === true || el.getAttribute("aria-disabled") === "true";
    var readonly = el.readOnly === true || el.getAttribute("aria-readonly") === "true";
    var editable = (tag === "textarea" || (tag === "input" && EDITABLE_TYPES.indexOf(type) !== -1)) && !readonly;
    out.push({
      handle: handle,
      role: roleOf(el),
      name: textOf(el),
      value: el.value !== undefined ? el.value : null,
      test_id: el.getAttribute("data-testid") || el.getAttribute("data-test-id") || el.getAttribute("data-test") || null,
      rect: { x: r.x, y: r.y, w: r.width, h: r.height },
      visible: visibleOf(el, r),
      enabled: !disabled,
      editable: editable,
      degraded: false
    });
  }
  return JSON.stringify(out);
})()
"#;

#[derive(Debug, Deserialize)]
struct SnapshotRect {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default, alias = "w")]
    width: f64,
    #[serde(default, alias = "h")]
    height: f64,
}

#[derive(Debug, Deserialize)]
struct SnapshotElement {
    handle: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    test_id: Option<String>,
    #[serde(default)]
    rect: Option<SnapshotRect>,
    #[serde(default)]
    visible: bool,
    #[serde(default = "snapshot_default_true")]
    enabled: bool,
    #[serde(default)]
    editable: bool,
    #[serde(default)]
    degraded: bool,
}

fn snapshot_default_true() -> bool {
    true
}

fn round_to_i32(value: f64) -> i32 {
    value.round() as i32
}

impl From<SnapshotElement> for InteractiveElement {
    fn from(element: SnapshotElement) -> Self {
        let value = element
            .value
            .filter(|value| !value.is_empty());
        InteractiveElement {
            element_id: element.handle,
            role: element.role,
            name: element.name,
            value,
            test_id: element.test_id.filter(|id| !id.is_empty()),
            bbox: element.rect.map(|rect| ElementBox {
                x: round_to_i32(rect.x),
                y: round_to_i32(rect.y),
                width: round_to_i32(rect.width),
                height: round_to_i32(rect.height),
            }),
            visible: element.visible,
            enabled: element.enabled,
            editable: element.editable,
            degraded: element.degraded,
        }
    }
}

/// Parse the [`GEOMETRY_SNAPSHOT_SCRIPT`] JSON output into a [`PageState`] whose
/// `interactive_elements` carry the stamped `data-theorem-id` as `element_id`
/// and the `getBoundingClientRect` as `bbox`. This is the Servo-free half of E4:
/// the live driver runs the script, this parses the result, and Codex's
/// [`Locator::resolve`](crate::browser_automation::Locator::resolve) then filters
/// the handles unchanged.
pub fn page_state_from_snapshot_json(url: &str, json: &str) -> BrowserEngineResult<PageState> {
    let elements: Vec<SnapshotElement> =
        serde_json::from_str(json).map_err(|error| BrowserEngineError::UnsupportedAction {
            reason: format!("geometry snapshot JSON did not parse: {error}"),
        })?;
    Ok(PageState {
        url: url.to_string(),
        title: String::new(),
        distilled_text: String::new(),
        interactive_elements: elements.into_iter().map(InteractiveElement::from).collect(),
        active_tab_id: None,
        fetch: None,
    })
}

// ---------------------------------------------------------------------------
// The actuation model. CoordinateSynthesis (V1) + SemanticActivation (#4344).
// ---------------------------------------------------------------------------

/// The pointer gesture a coordinate-synthesis plan carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerKind {
    Click,
    DoubleClick,
    Hover,
    Tap,
}

/// The native control a `<select>` or file input resolves to (mechanism B).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EmbedderControlPlan {
    SelectOption { value: String },
    SetInputFiles { paths: Vec<String> },
}

/// The semantic action a #4344 activation requests once the Servo fork forwards
/// AccessKit `ActionRequest`s into DOM activation. Defined now so the seam is
/// stable; only the forked Servo driver executes it (V1 drivers return
/// `UnsupportedAction`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAction {
    Click,
    Focus,
}

/// How an action is actuated against the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mechanism")]
pub enum ActuationKind {
    /// Mechanism A (V1): synthesize input events at a Paint-hit-tested device
    /// point via `notify_input_event`.
    CoordinateSynthesis { point: DevicePoint, pointer: PointerKind },
    /// Focus by coordinate, then commit text (keyboard / IME).
    Keyboard { point: DevicePoint, text: String },
    /// Mechanism B: respond to a Servo-rendered native control.
    EmbedderControl { control: EmbedderControlPlan },
    /// Scroll the element into view, then re-measure.
    Scroll { point: DevicePoint },
    /// The #4344 route: activate the element by AccessKit node id. The engine
    /// phase fills this in; V1 drivers reject it.
    SemanticActivation { node_id: u64, action: SemanticAction },
}

/// A resolved actuation: the gesture plus the handle it targets (so a driver
/// without geometry, like the fetch-cascade engine, can map back by handle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActuationPlan {
    pub target_handle: String,
    pub kind: ActuationKind,
}

/// What the driver reports after actuating: the mechanism it used and a
/// structured receipt (the `notify_input_event_handled` result for the live
/// driver, the fetch-cascade receipt for the test path).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActuationReceipt {
    pub mechanism: String,
    pub detail: Value,
}

// ---------------------------------------------------------------------------
// The driver trait: the minimal Servo I/O surface the contract needs.
// ---------------------------------------------------------------------------

/// The seam between the Servo-free Playwright-class contract and a backend.
///
/// A backend supplies three things: the current actionable [`PageState`]
/// (`snapshot`), the coordinate space (`device_pixels_per_css_pixel` /
/// `webview_origin`), and execution (`actuate`). The live Servo driver derives
/// `snapshot` from one `evaluate_javascript(GEOMETRY_SNAPSHOT_SCRIPT)` call plus
/// [`page_state_from_snapshot_json`]; the fetch-cascade driver returns its
/// HTML-parse `PageState`.
#[allow(async_fn_in_trait)]
pub trait BrowserDriver {
    /// The current actionable snapshot (E4).
    fn snapshot(&self) -> BrowserEngineResult<PageState>;

    /// Page zoom + pinch + HiDPI scale; 1.0 for a driver without geometry.
    fn device_pixels_per_css_pixel(&self) -> f32 {
        1.0
    }

    /// The webview origin in the rendering context (E6); zero for the test path.
    fn webview_origin(&self) -> DevicePoint {
        DevicePoint::ZERO
    }

    /// Execute a resolved actuation plan.
    async fn actuate(
        &mut self,
        plan: ActuationPlan,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<ActuationReceipt>;
}

/// Build the actuation plan for a gated handle: compute the device point from the
/// handle rect through the E5/E6 transform, then pick the mechanism per action.
pub fn build_actuation_plan<D: BrowserDriver + ?Sized>(
    driver: &D,
    handle: &ElementHandle,
    action: &LocatorAction,
) -> BrowserEngineResult<ActuationPlan> {
    let point = handle
        .rect
        .as_ref()
        .map(|rect| {
            device_point_at_rect_center(
                rect,
                driver.device_pixels_per_css_pixel(),
                driver.webview_origin(),
            )
        })
        .unwrap_or(DevicePoint::ZERO);
    let kind = match action {
        LocatorAction::Click | LocatorAction::Check | LocatorAction::SetChecked { .. } => {
            ActuationKind::CoordinateSynthesis {
                point,
                pointer: PointerKind::Click,
            }
        }
        LocatorAction::DoubleClick => ActuationKind::CoordinateSynthesis {
            point,
            pointer: PointerKind::DoubleClick,
        },
        LocatorAction::Tap => ActuationKind::CoordinateSynthesis {
            point,
            pointer: PointerKind::Tap,
        },
        LocatorAction::Hover => ActuationKind::CoordinateSynthesis {
            point,
            pointer: PointerKind::Hover,
        },
        LocatorAction::Fill { value } => ActuationKind::Keyboard {
            point,
            text: value.clone(),
        },
        LocatorAction::ScrollIntoView => ActuationKind::Scroll { point },
        LocatorAction::SelectOption { value } => ActuationKind::EmbedderControl {
            control: EmbedderControlPlan::SelectOption {
                value: value.clone(),
            },
        },
        LocatorAction::SetInputFiles { paths } => ActuationKind::EmbedderControl {
            control: EmbedderControlPlan::SetInputFiles {
                paths: paths.clone(),
            },
        },
    };
    Ok(ActuationPlan {
        target_handle: handle.handle.clone(),
        kind,
    })
}

// ---------------------------------------------------------------------------
// The auto-wait action runner.
// ---------------------------------------------------------------------------

/// Resolve `locator`, gate it, and actuate - retrying on the snapshot cadence
/// until the actionability gate passes or `options.timeout_ms` elapses. This is
/// the Playwright auto-wait that the one-shot `perform_locator_action` does not
/// do: a field that is briefly disabled then enabled succeeds without a sleep,
/// and a click that never receives events fails closed at the deadline rather
/// than firing blind.
///
/// Returns `ElementNotFound` if the locator never resolves by the deadline;
/// returns an unapplied receipt (with the last failing verdict) if it resolves
/// but the gate never passes.
pub async fn run_action<D: BrowserDriver>(
    driver: &mut D,
    locator: &Locator,
    action: LocatorAction,
    options: ActionOptions,
    policy: &BrowserActionPolicy,
) -> BrowserEngineResult<AutomationActionReceipt> {
    let requirement = ActionabilityRequirement::for_action(&action, options.force);
    let deadline = Instant::now() + Duration::from_millis(options.timeout_ms);
    let poll = Duration::from_millis(SNAPSHOT_POLL_MS);
    let mut attempts = 0usize;
    let mut last: Option<(ElementHandle, ActionabilityVerdict)> = None;

    loop {
        attempts += 1;
        let page = driver.snapshot()?;
        if let Some(handle) = locator.resolve(&page).into_iter().next() {
            let mut verdict = ActionabilityVerdict::evaluate(&handle, &requirement);
            verdict.attempts = attempts;
            if verdict.passed {
                let plan = build_actuation_plan(driver, &handle, &action)?;
                let actuation = driver.actuate(plan, policy).await?;
                return Ok(AutomationActionReceipt {
                    action,
                    selector: locator.selector_summary(),
                    handle,
                    actionability: verdict,
                    applied: true,
                    browser_action: None,
                    engine_receipt: Some(json!({
                        "mechanism": actuation.mechanism,
                        "detail": actuation.detail,
                    })),
                });
            }
            last = Some((handle, verdict));
        }

        if Instant::now() >= deadline {
            return match last {
                Some((handle, verdict)) => Ok(AutomationActionReceipt {
                    action,
                    selector: locator.selector_summary(),
                    handle,
                    actionability: verdict,
                    applied: false,
                    browser_action: None,
                    engine_receipt: None,
                }),
                None => Err(BrowserEngineError::ElementNotFound {
                    element_id: locator.selector_summary(),
                }),
            };
        }
        sleep(poll).await;
    }
}

// ---------------------------------------------------------------------------
// The fast-path driver: the fetch-cascade engine behind the same seam.
// ---------------------------------------------------------------------------

/// Map a resolved plan back to the fetch-cascade `BrowserAction` vocabulary by
/// handle. The fetch-cascade engine has no geometry, so it ignores the device
/// point and actuates by `element_id`; semantic activation has no meaning
/// without the Servo fork.
fn browser_action_for_plan(plan: &ActuationPlan) -> BrowserEngineResult<BrowserAction> {
    let element_id = plan.target_handle.clone();
    match &plan.kind {
        ActuationKind::CoordinateSynthesis { pointer, .. } => match pointer {
            PointerKind::Hover => Ok(BrowserAction::Hover { element_id }),
            _ => Ok(BrowserAction::Click { element_id }),
        },
        ActuationKind::Keyboard { text, .. } => Ok(BrowserAction::Type {
            element_id,
            text: text.clone(),
        }),
        ActuationKind::Scroll { .. } => Ok(BrowserAction::ScrollToElement { element_id }),
        ActuationKind::EmbedderControl { control } => match control {
            EmbedderControlPlan::SelectOption { value } => Ok(BrowserAction::SelectOption {
                element_id,
                value: value.clone(),
            }),
            EmbedderControlPlan::SetInputFiles { paths } => {
                let path = paths.first().ok_or_else(|| BrowserEngineError::UnsupportedAction {
                    reason: "set_input_files requires at least one path".to_string(),
                })?;
                Ok(BrowserAction::UploadFile {
                    element_id,
                    path: path.clone(),
                })
            }
        },
        ActuationKind::SemanticActivation { .. } => Err(BrowserEngineError::UnsupportedAction {
            reason: "semantic activation (#4344) needs the Servo fork; the fetch-cascade \
                     driver actuates by coordinate synthesis only"
                .to_string(),
        }),
    }
}

impl BrowserDriver for FetchCascadeBrowserEngine {
    fn snapshot(&self) -> BrowserEngineResult<PageState> {
        self.observe()
    }

    async fn actuate(
        &mut self,
        plan: ActuationPlan,
        policy: &BrowserActionPolicy,
    ) -> BrowserEngineResult<ActuationReceipt> {
        let action = browser_action_for_plan(&plan)?;
        let BrowserActionOutcome { receipt, .. } = self.act(action, policy).await?;
        Ok(ActuationReceipt {
            mechanism: "fetch_cascade".to_string(),
            detail: receipt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_automation::{ActionabilityCheck, RoleOptions};

    /// A driver whose snapshot can change across calls, so the auto-wait loop is
    /// testable without a live engine or wall-clock games: the loop re-snapshots
    /// and converges (or hits the deadline) deterministically.
    struct FakeDriver {
        frames: Vec<PageState>,
        calls: std::cell::Cell<usize>,
        dppx: f32,
        origin: DevicePoint,
        actuated: std::cell::RefCell<Vec<ActuationPlan>>,
    }

    impl FakeDriver {
        fn new(frames: Vec<PageState>, dppx: f32, origin: DevicePoint) -> Self {
            Self {
                frames,
                calls: std::cell::Cell::new(0),
                dppx,
                origin,
                actuated: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl BrowserDriver for FakeDriver {
        fn snapshot(&self) -> BrowserEngineResult<PageState> {
            let index = self.calls.get();
            self.calls.set(index + 1);
            let frame = self
                .frames
                .get(index)
                .or_else(|| self.frames.last())
                .cloned()
                .ok_or(BrowserEngineError::NoCurrentPage)?;
            Ok(frame)
        }

        fn device_pixels_per_css_pixel(&self) -> f32 {
            self.dppx
        }

        fn webview_origin(&self) -> DevicePoint {
            self.origin
        }

        async fn actuate(
            &mut self,
            plan: ActuationPlan,
            _policy: &BrowserActionPolicy,
        ) -> BrowserEngineResult<ActuationReceipt> {
            self.actuated.borrow_mut().push(plan.clone());
            Ok(ActuationReceipt {
                mechanism: "fake".to_string(),
                detail: json!({ "target": plan.target_handle }),
            })
        }
    }

    fn element(
        id: &str,
        role: &str,
        name: &str,
        visible: bool,
        enabled: bool,
        editable: bool,
        degraded: bool,
        rect: Option<ElementBox>,
    ) -> InteractiveElement {
        InteractiveElement {
            element_id: id.to_string(),
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            test_id: None,
            bbox: rect,
            visible,
            enabled,
            editable,
            degraded,
        }
    }

    fn page(elements: Vec<InteractiveElement>) -> PageState {
        PageState {
            url: "https://example.com/".to_string(),
            title: String::new(),
            distilled_text: String::new(),
            interactive_elements: elements,
            active_tab_id: None,
            fetch: None,
        }
    }

    #[test]
    fn snapshot_json_parses_into_stamped_handles_and_rects() {
        let json = r#"[
            {"handle":"t0","role":"button","name":"Save","value":null,"test_id":"save",
             "rect":{"x":10,"y":20,"w":80,"h":24},"visible":true,"enabled":true,"editable":false},
            {"handle":"t1","role":"text","name":"q","value":"","rect":{"x":0,"y":60,"w":200,"h":30},
             "visible":true,"enabled":false,"editable":true}
        ]"#;
        let page = page_state_from_snapshot_json("https://example.com/", json).expect("parse");
        assert_eq!(page.interactive_elements.len(), 2);
        // Option A: the actionable id is the data-theorem-id stamp.
        assert_eq!(page.interactive_elements[0].element_id, "t0");
        assert_eq!(
            page.interactive_elements[0].bbox,
            Some(ElementBox { x: 10, y: 20, width: 80, height: 24 })
        );
        assert_eq!(page.interactive_elements[0].test_id.as_deref(), Some("save"));
        // Empty value strings collapse to None (parity with the HTML reader).
        assert!(page.interactive_elements[1].value.is_none());
        assert!(!page.interactive_elements[1].enabled);
        assert!(page.interactive_elements[1].editable);
    }

    #[test]
    fn coordinate_transform_lands_device_point_at_rect_center() {
        let rect = ElementBox { x: 100, y: 40, width: 60, height: 20 };
        // center css = (130, 50); dppx 2.0; origin (10, 5) => (270, 105).
        let point = device_point_at_rect_center(&rect, 2.0, DevicePoint { x: 10.0, y: 5.0 });
        assert_eq!(point, DevicePoint { x: 270.0, y: 105.0 });
    }

    #[test]
    fn snapshot_script_shares_provenance_with_the_selector_bridge() {
        // The snapshot owns role/name semantics; the borrowed selector bridge is
        // the targeted-query helper. Both compute role from the same tag table.
        assert!(GEOMETRY_SNAPSHOT_SCRIPT.contains("data-theorem-id"));
        assert!(GEOMETRY_SNAPSHOT_SCRIPT.contains("getBoundingClientRect"));
        assert!(crate::browser_automation::SELECTOR_BRIDGE_SCRIPT.contains("roleOf"));
    }

    #[tokio::test]
    async fn auto_wait_passes_once_a_briefly_disabled_field_enables() {
        // Frame 0: the field is disabled. Frame 1: enabled. The loop must
        // re-snapshot and succeed on the second attempt without a sleep call.
        let disabled = page(vec![element(
            "t0", "text", "q", true, false, true, false,
            Some(ElementBox { x: 0, y: 0, width: 100, height: 20 }),
        )]);
        let enabled = page(vec![element(
            "t0", "text", "q", true, true, true, false,
            Some(ElementBox { x: 0, y: 0, width: 100, height: 20 }),
        )]);
        let mut driver = FakeDriver::new(vec![disabled, enabled], 1.0, DevicePoint::ZERO);

        let receipt = run_action(
            &mut driver,
            &Locator::get_by_label("q"),
            LocatorAction::Fill { value: "servo".to_string() },
            ActionOptions { timeout_ms: 1_000, force: false },
            &BrowserActionPolicy::default(),
        )
        .await
        .expect("fill");

        assert!(receipt.applied);
        assert!(receipt.actionability.passed);
        assert!(receipt.actionability.attempts >= 2);
        assert_eq!(driver.actuated.borrow().len(), 1);
        assert!(matches!(
            driver.actuated.borrow()[0].kind,
            ActuationKind::Keyboard { .. }
        ));
    }

    #[tokio::test]
    async fn auto_wait_fails_closed_when_receives_events_never_passes() {
        // A degraded button never receives events; clicking must fail closed at
        // the deadline (not fire blind), with the gating check reported.
        let occluded = page(vec![element(
            "t0", "button", "Save", true, true, false, true,
            Some(ElementBox { x: 0, y: 0, width: 80, height: 24 }),
        )]);
        let mut driver = FakeDriver::new(vec![occluded], 1.0, DevicePoint::ZERO);

        let receipt = run_action(
            &mut driver,
            &Locator::get_by_role("button", RoleOptions { name: Some("Save".to_string()) }),
            LocatorAction::Click,
            ActionOptions { timeout_ms: 60, force: false },
            &BrowserActionPolicy::default(),
        )
        .await
        .expect("receipt");

        assert!(!receipt.applied);
        assert!(receipt
            .actionability
            .missing
            .contains(&ActionabilityCheck::ReceivesEvents));
        assert!(receipt.actionability.attempts >= 1);
        assert!(driver.actuated.borrow().is_empty());
    }

    #[tokio::test]
    async fn force_clicks_a_degraded_button_through_the_gate() {
        let occluded = page(vec![element(
            "t0", "button", "Save", true, true, false, true,
            Some(ElementBox { x: 0, y: 0, width: 80, height: 24 }),
        )]);
        let mut driver = FakeDriver::new(vec![occluded], 1.0, DevicePoint::ZERO);

        let receipt = run_action(
            &mut driver,
            &Locator::get_by_role("button", RoleOptions { name: Some("Save".to_string()) }),
            LocatorAction::Click,
            ActionOptions { timeout_ms: 1_000, force: true },
            &BrowserActionPolicy::default(),
        )
        .await
        .expect("forced click");

        assert!(receipt.applied);
        assert!(matches!(
            driver.actuated.borrow()[0].kind,
            ActuationKind::CoordinateSynthesis { pointer: PointerKind::Click, .. }
        ));
    }

    #[tokio::test]
    async fn never_attached_locator_times_out_as_element_not_found() {
        let empty = page(vec![]);
        let mut driver = FakeDriver::new(vec![empty], 1.0, DevicePoint::ZERO);

        let error = run_action(
            &mut driver,
            &Locator::get_by_test_id("ghost"),
            LocatorAction::Click,
            ActionOptions { timeout_ms: 50, force: false },
            &BrowserActionPolicy::default(),
        )
        .await
        .expect_err("should time out");

        assert!(matches!(error, BrowserEngineError::ElementNotFound { .. }));
    }

    #[tokio::test]
    async fn semantic_activation_is_rejected_by_a_coordinate_only_driver() {
        let occluded = page(vec![element(
            "t0", "button", "Save", true, true, false, false,
            Some(ElementBox { x: 0, y: 0, width: 80, height: 24 }),
        )]);
        let mut driver = FakeDriver::new(vec![occluded], 1.0, DevicePoint::ZERO);
        // The #4344 variant is structurally present; a coordinate-only driver
        // maps it through, and the fetch-cascade engine would reject it. Here we
        // assert the plan/mechanism shape exists and round-trips through serde.
        let plan = ActuationPlan {
            target_handle: "t0".to_string(),
            kind: ActuationKind::SemanticActivation {
                node_id: 7,
                action: SemanticAction::Click,
            },
        };
        let round_trip: ActuationPlan =
            serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(round_trip, plan);
        // The fake driver records it; the fetch-cascade map-back rejects it.
        assert!(browser_action_for_plan(&plan).is_err());
        let _ = driver
            .actuate(plan, &BrowserActionPolicy::default())
            .await
            .expect("fake driver accepts any plan");
    }
}
