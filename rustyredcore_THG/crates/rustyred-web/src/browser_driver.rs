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

use crate::browser_engine::{
    BrowserActionOutcome, BrowserActionPolicy, BrowserEngineResult, FetchCascadeBrowserEngine,
    PageState,
};

// The Servo-free driver contract (the `BrowserDriver` trait, `run_action`'s
// auto-wait loop, the geometry snapshot, the E5/E6 transform, `ActuationKind`,
// `browser_action_for_plan`) lives in `pilot_core::driver` (migrated in place
// toward an open-source "WebDriver BiDi for Servo"); re-exported so consumers and
// the apps/browser adapter are unchanged. rustyred-web keeps only the
// fetch-cascade `BrowserDriver` impl below.
pub use pilot_core::driver::*;
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
