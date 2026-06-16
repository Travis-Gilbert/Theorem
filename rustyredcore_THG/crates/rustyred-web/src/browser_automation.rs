//! Playwright-shaped automation core for the substrate browser.
//!
//! This module is Servo-free by design. It defines the locator, actionability,
//! context, routing, assertion, and receipt contracts that a live Servo embedder
//! can satisfy with `evaluate_javascript` and input synthesis, while the
//! existing fetch-cascade engine can exercise the same API in fast unit tests.

use crate::browser_engine::{
    BrowserActionOutcome, BrowserActionPolicy, BrowserEngineError, BrowserEngineResult,
    FetchCascadeBrowserEngine,
};

// The Servo-free Playwright-class contract (Locator, actionability, Context,
// expect, receipts, the vendored selector bridge) now lives in
// `pilot_core::automation` (migrated in place toward an open-source "WebDriver
// BiDi for Servo"); re-exported so consumers and the fetch-cascade glue +
// tests below are unchanged.
pub use pilot_core::automation::*;

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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_engine::{page_state_from_html, BrowserAction, PageState};
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
