use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::browser_engine::{PageState, WebConsumeReceipt};

pub use crate::browser_engine::InteractiveElement;

pub const BEHAVIOR_TEXT_PREVIEW_CHARS: usize = 280;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WebBehaviorObservation {
    pub url: String,
    pub title: String,
    pub route: ObservedRoute,
    pub forms: Vec<ObservedForm>,
    pub inputs: Vec<ObservedInteraction>,
    pub buttons: Vec<ObservedInteraction>,
    pub links: Vec<ObservedInteraction>,
    pub distilled_text_hash: String,
    pub distilled_text_preview: String,
    pub outbound_links: Vec<String>,
    pub outbound_domain_summary: Vec<DomainSummary>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedRoute {
    pub url: String,
    pub path: String,
    pub domain: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedForm {
    pub form_id: String,
    pub controls: Vec<ObservedInteraction>,
    pub unknowns: Vec<String>,
    pub inferred: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservedInteraction {
    pub element_id: String,
    pub kind: String,
    pub name: String,
    pub value: Option<String>,
    pub visible: bool,
    pub enabled: bool,
    pub editable: bool,
    pub test_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DomainSummary {
    pub domain: String,
    pub link_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BehaviorObservationBundle {
    pub observations: Vec<WebBehaviorObservation>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum BehaviorInput {
    Receipt(WebConsumeReceipt),
    PageState(PageState),
}

impl From<WebConsumeReceipt> for BehaviorInput {
    fn from(receipt: WebConsumeReceipt) -> Self {
        Self::Receipt(receipt)
    }
}

impl From<PageState> for BehaviorInput {
    fn from(page: PageState) -> Self {
        Self::PageState(page)
    }
}

pub fn observations_from_inputs<I>(inputs: I) -> BehaviorObservationBundle
where
    I: IntoIterator<Item = BehaviorInput>,
{
    let mut unknowns = BTreeSet::new();
    let mut observations = Vec::new();

    for input in inputs {
        let page = match input {
            BehaviorInput::Receipt(receipt) => receipt.page,
            BehaviorInput::PageState(page) => page,
        };
        let observation = behavior_observation_from_page(page);
        unknowns.extend(observation.unknowns.iter().cloned());
        observations.push(observation);
    }

    BehaviorObservationBundle {
        observations,
        unknowns: unknowns.into_iter().collect(),
    }
}

pub fn observations_from_pages<I>(pages: I) -> BehaviorObservationBundle
where
    I: IntoIterator<Item = PageState>,
{
    observations_from_inputs(pages.into_iter().map(BehaviorInput::PageState))
}

pub fn observations_from_receipts<I>(receipts: I) -> BehaviorObservationBundle
where
    I: IntoIterator<Item = WebConsumeReceipt>,
{
    observations_from_inputs(receipts.into_iter().map(BehaviorInput::Receipt))
}

fn behavior_observation_from_page(page: PageState) -> WebBehaviorObservation {
    let route = route_from_url(&page.url);
    let distilled_text_hash = blake3::hash(page.distilled_text.as_bytes())
        .to_hex()
        .to_string();
    let distilled_text_preview = preview_text(&page.distilled_text, BEHAVIOR_TEXT_PREVIEW_CHARS);

    let mut inputs = Vec::new();
    let mut buttons = Vec::new();
    let mut links = Vec::new();
    let mut form_inputs = Vec::new();
    let mut form_unknowns = Vec::new();
    let mut outbound_links = Vec::new();
    let mut domain_counts = BTreeMap::<String, usize>::new();
    let page_domain = route.domain.clone();

    for element in page.interactive_elements {
        let observed = observed_interaction_from_element(&element);
        match interaction_kind(&element.role) {
            InteractionClass::Link => {
                if let Some(value) = &observed.value {
                    if let Some(domain) = link_domain(value) {
                        if page_domain.as_deref() != Some(domain.as_str()) {
                            outbound_links.push(value.clone());
                            *domain_counts.entry(domain).or_insert(0) += 1;
                        }
                    }
                } else {
                    form_unknowns.push(format!("link_without_dest:element:{}", element.element_id));
                }
                links.push(observed);
            }
            InteractionClass::Input => {
                inputs.push(observed.clone());
                form_inputs.push(observed);
            }
            InteractionClass::Button => {
                buttons.push(observed.clone());
                form_inputs.push(observed);
            }
            InteractionClass::Unknown(role) => {
                form_unknowns.push(format!("unknown_interaction_role:{role}"));
            }
        }
    }

    let forms = if !inputs.is_empty() || !buttons.is_empty() {
        vec![ObservedForm {
            form_id: "inferred_form_0".to_string(),
            controls: form_inputs,
            unknowns: form_unknowns.clone(),
            inferred: true,
        }]
    } else {
        Vec::new()
    };

    let outbound_links = unique_sorted(outbound_links);
    let outbound_domain_summary = domain_counts
        .into_iter()
        .map(|(domain, link_count)| DomainSummary { domain, link_count })
        .collect();

    let mut unknowns = Vec::new();
    unknowns.extend(form_unknowns);

    WebBehaviorObservation {
        url: page.url,
        title: page.title,
        route,
        forms,
        inputs,
        buttons,
        links,
        distilled_text_hash,
        distilled_text_preview,
        outbound_links,
        outbound_domain_summary,
        unknowns,
    }
}

#[derive(Clone, Debug)]
enum InteractionClass {
    Link,
    Input,
    Button,
    Unknown(String),
}

fn interaction_kind(role: &str) -> InteractionClass {
    match role {
        "link" => InteractionClass::Link,
        "button" => InteractionClass::Button,
        "select" | "textbox" => InteractionClass::Input,
        "text" | "search" | "email" | "password" | "url" | "tel" | "number" | "date"
        | "datetime-local" | "month" | "time" | "week" | "hidden" | "checkbox" | "radio"
        | "file" | "range" | "color" | "datetime" | "submit" | "reset" | "input" => {
            InteractionClass::Input
        }
        other => InteractionClass::Unknown(other.to_string()),
    }
}

fn observed_interaction_from_element(element: &InteractiveElement) -> ObservedInteraction {
    ObservedInteraction {
        element_id: element.element_id.clone(),
        kind: element.role.clone(),
        name: element.name.clone(),
        value: element.value.clone(),
        visible: element.visible,
        enabled: element.enabled,
        editable: element.editable,
        test_id: element.test_id.clone(),
    }
}

fn route_from_url(url: &str) -> ObservedRoute {
    match Url::parse(url) {
        Ok(parsed) => ObservedRoute {
            url: url.to_string(),
            path: parsed.path().to_string(),
            domain: parsed.host_str().map(|host| host.to_string()),
        },
        Err(_) => ObservedRoute {
            url: url.to_string(),
            path: "/".to_string(),
            domain: None,
        },
    }
}

fn link_domain(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
}

fn unique_sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.trim();
    if normalized.chars().count() <= max_chars {
        return normalized.to_string();
    }
    normalized
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_state_from_html;

    #[test]
    fn extracts_route_forms_buttons_inputs_and_links_from_page_state() {
        let page = page_state_from_html(
            "https://example.com/checkout?step=1",
            r#"
                <html>
                  <head><title>Checkout</title></head>
                  <body>
                    <a href="/shipping">Shipping</a>
                    <a href="https://partner.example/lead">Partner</a>
                    <button id="continue">Continue</button>
                    <input type="text" name="email" value="user@example.com" />
                    <input type="checkbox" name="tos" checked />
                    <textarea name="notes"></textarea>
                    <select name="plan"><option>basic</option></select>
                  </body>
                </html>
            "#,
        )
        .expect("page");
        let bundle = observations_from_pages(vec![page]);

        assert_eq!(bundle.observations.len(), 1);
        let observation = &bundle.observations[0];
        assert_eq!(observation.url, "https://example.com/checkout?step=1");
        assert_eq!(observation.title, "Checkout");
        assert_eq!(observation.route.path, "/checkout");
        assert_eq!(
            observation.route.domain.as_deref(),
            Some("example.com"),
            "route domain should be factual, not inferred"
        );
        assert_eq!(observation.links.len(), 2);
        assert_eq!(observation.buttons.len(), 1);
        assert_eq!(observation.inputs.len(), 4);
        assert!(observation.distilled_text_preview.contains("Checkout"));
        assert_eq!(observation.distilled_text_hash.len(), 64);
        assert_eq!(
            observation.outbound_links,
            vec!["https://partner.example/lead".to_string()]
        );
        assert_eq!(observation.outbound_domain_summary.len(), 1);
        assert!(observation.forms.len() >= 1);
        assert_eq!(observation.forms[0].form_id, "inferred_form_0");
        assert!(observation.forms[0].inferred);
        assert!(observation.unknowns.is_empty());
    }

    #[test]
    fn preserves_unknown_interaction_roles_without_business_inference() {
        let mut page = page_state_from_html(
            "https://example.com/",
            "<html><body><a href=\"/\">Home</a></body></html>",
        )
        .expect("page");
        page.interactive_elements.push(InteractiveElement {
            element_id: "custom_0".to_string(),
            role: "mystery-widget".to_string(),
            name: "widget".to_string(),
            value: Some("on".to_string()),
            test_id: Some("widget".to_string()),
            bbox: None,
            visible: true,
            enabled: true,
            editable: false,
            degraded: false,
        });

        let bundle = observations_from_pages(vec![page]);

        let observation = &bundle.observations[0];
        assert_eq!(
            observation.unknowns,
            vec!["unknown_interaction_role:mystery-widget"]
        );
        assert_eq!(
            bundle.unknowns,
            vec!["unknown_interaction_role:mystery-widget"]
        );
        assert!(observation.forms.is_empty());
        assert_eq!(observation.links.len(), 1);
        assert_eq!(observation.inputs.len(), 0);
        assert_eq!(observation.buttons.len(), 0);
        assert!(observation.outbound_links.is_empty());
    }
}
