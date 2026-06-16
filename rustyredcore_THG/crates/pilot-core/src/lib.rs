//! `pilot-core`: a Servo-free, Playwright-class browser automation core.
//!
//! This crate holds the entire automation contract with lean dependencies
//! (serde + tokio only) and **zero substrate coupling**, so one set of logic can
//! drive any backend through the [`BrowserDriver`] trait: a live `servo::WebView`
//! (the `apps/browser` adapter), the fetch-cascade engine (rustyred-web), a fake
//! driver (tests), and -- later -- a WebDriver BiDi front end that lets real
//! Playwright / Puppeteer / WebdriverIO clients drive Servo.
//!
//! The differentiation over CDP-class tooling is that actionability is computed
//! from engine truth (box tree, Paint hit-testing, frame-accurate settle) rather
//! than injected DOM heuristics. Playwright's reliability is auto-wait; Servo can
//! do auto-wait better because it owns the layout and the frame loop.
//!
//! Status: scaffold. Content is being migrated **in place** from rustyred-web
//! (`browser_driver.rs`, `browser_automation.rs`, and the data-type half of
//! `browser_engine.rs`) per
//! `docs/plans/servo-browser-use-agent/pilot-core-extraction.md`. Until the move
//! completes, rustyred-web remains the home and this crate is empty by design.
//!
//! Falsifiable boundary check: `cargo tree -p pilot-core` must show no
//! `rustyred_thg_core`, no `ndarray`, no `cblas-sys`, no `openblas`.

// Modules land here as the migration proceeds (see the plan's slices):
//   pub mod types;         // ElementBox, InteractiveElement, PageState, BrowserAction, ...
//   pub mod driver;        // BrowserDriver trait, ActuationKind, DevicePoint, E5/E6 transform
//   pub mod snapshot;      // GEOMETRY_SNAPSHOT_SCRIPT + page_state_from_snapshot_json (Option A)
//   pub mod locator;       // Locator + resolution
//   pub mod actionability; // the six checks + the per-action matrix + run_action auto-wait
//   pub mod context;       // BrowserContext storage/permission partition + route()
//   pub mod assertion;     // web-first expect()
//   pub mod trace;         // the run recorder (actionability verdicts + action log)

pub mod types;
pub use types::{
    BrowserAction, BrowserActionPolicy, BrowserEngineError, BrowserEngineResult, ElementBox,
    InteractiveElement, PageState, WaitCondition,
};

pub mod masking;
pub use masking::{MaskedText, SensitiveData};

pub mod automation;
pub use automation::*;

pub mod driver;
pub use driver::*;
