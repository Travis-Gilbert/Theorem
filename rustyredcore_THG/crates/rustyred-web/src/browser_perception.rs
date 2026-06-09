//! Servo browser-use, build step one (job-007): the perception + governance half.
//!
//! This module is the reader-and-governance seam of the parity slice. The
//! executor half (the engine-native `act()` path through AccessKit
//! `ActionRequest`, the parity `BrowserAction` variants, upload, and tab
//! webview control) lives in [`crate::browser_engine`] and is owned by the
//! other head. This module produces what the model perceives and the gates the
//! engine enforces; the executor consumes both.
//!
//! What it carries, mapped to the job-007 deliverables:
//! - D1: [`AccessibilityReader`], a *stateful* reader that consumes the Servo
//!   accessibility tree (projected as [`A11yTreeUpdate`]) and produces the
//!   [`PageState`] contract. The element id is the accesskit `NodeId` (a `u64`),
//!   so there is no second id space (the standing fence). Incremental
//!   `TreeUpdate`s update the page without a full re-walk: this is the
//!   engine-native equivalent of a mutation observer.
//! - D3: [`validate_against_schema`] / [`extract_structured`] (structured-output
//!   validation), [`resolve_upload_path`] (upload path allowlist), [`TabSet`]
//!   (the tab model).
//! - D4: [`DomainPolicy`] (domain restriction), [`SensitiveData`] (domain-scoped
//!   secret masking), [`detect_download`] (download detection).
//! - D5: the `degraded` marking on [`InteractiveElement`] plus
//!   [`keyboard_fallback_for`] (the keyboard-fallback plan the executor runs).
//!
//! ## The accesskit boundary (intentional, documented)
//!
//! The reader consumes [`A11yTreeUpdate`], a faithful serde projection of
//! `accesskit::{TreeUpdate, Node, NodeId, Rect}`, rather than depending on the
//! `accesskit` crate in the agent layer by default. This mirrors the existing
//! `apps/browser-substrate` seam (Servo `LoadedPage` -> `rustyred-web`): the
//! embedder, which already links accesskit via Servo, owns the trivial
//! `accesskit::TreeUpdate -> A11yTreeUpdate` mapping, and the agent layer stays
//! free of Servo's accesskit version pin. The real conversion is provided under
//! the optional `accesskit` feature (see [`A11yTreeUpdate::from_accesskit`]) and
//! is the function the embedder calls; the pure DTO keeps the reader fully
//! unit-testable with no engine present.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::browser_engine::{ElementBox, InteractiveElement, PageState};

// ===========================================================================
// D1: the AccessKit-projected accessibility tree (the reader input contract)
// ===========================================================================

/// A rectangle, the faithful projection of `accesskit::Rect` (`x0,y0,x1,y1`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct A11yRect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl A11yRect {
    pub fn width(&self) -> f64 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(&self) -> f64 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// Project to the engine `ElementBox` (integer device pixels) the rest of
    /// the `PageState` contract uses.
    pub fn to_element_box(&self) -> ElementBox {
        ElementBox {
            x: self.x0.round() as i32,
            y: self.y0.round() as i32,
            width: self.width().round() as i32,
            height: self.height().round() as i32,
        }
    }
}

/// A single accessibility node, the faithful projection of `accesskit::Node`
/// plus its `NodeId`. `role` is the accesskit `Role` variant name (e.g.
/// `"Button"`, `"TextInput"`, `"GenericContainer"`) so the agent layer never
/// has to enumerate the ~180 accesskit roles to stay in sync.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct A11yNode {
    /// The accesskit `NodeId` inner `u64`. This is the element id; do not mint a
    /// second id space.
    pub id: u64,
    /// The accesskit `Role` variant name.
    pub role: String,
    /// Accessible name (accesskit `label`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Current value / text content (accesskit `value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Layout bounds (accesskit `bounds`); absent until the engine populates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<A11yRect>,
    /// accesskit `is_hidden`.
    #[serde(default)]
    pub hidden: bool,
    /// accesskit `is_disabled`.
    #[serde(default)]
    pub disabled: bool,
    /// The node can receive focus (supports `Action::Focus`). Used to recognise
    /// an actionable node the engine has not given a proper interactive role.
    #[serde(default)]
    pub focusable: bool,
    /// The node has a default/click action (supports `Action::Click`/`Default`).
    #[serde(default)]
    pub supports_default_action: bool,
    /// Toggle state for checkbox/radio/switch: `Some(true|false)` toggled,
    /// `None` when not a toggle or indeterminate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggled: Option<bool>,
    /// Child node ids, in document/reading order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<u64>,
}

/// One atomic update to the accessibility tree, the faithful projection of
/// `accesskit::TreeUpdate`. As in accesskit, `nodes` carries only new or
/// changed nodes; a node is removed by no longer being referenced from a live
/// parent's `children`. `root`/`focus`/`url`/`title` are optional and only sent
/// when they change (`url`/`title` are supplied by the embedder, which knows the
/// loaded document; they are not accesskit tree properties).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct A11yTreeUpdate {
    #[serde(default)]
    pub nodes: Vec<A11yNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The change signal produced by applying a [`A11yTreeUpdate`]: which live nodes
/// were added, changed, or removed. This is the deterministic, layout-sourced
/// diff (job-008 D2 builds the post-action precise diff on top of it); for
/// job-007 it proves incremental updates without a full re-walk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct A11yDiff {
    pub added: Vec<u64>,
    pub changed: Vec<u64>,
    pub removed: Vec<u64>,
}

impl A11yDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

/// The stateful PageState reader over the Servo accessibility tree.
///
/// Holds the live node map and applies incremental `A11yTreeUpdate`s, then
/// reprojects the [`PageState`] contract. Reachability from `root` defines
/// liveness, so a node removed from its parent's `children` (the accesskit
/// removal contract) is pruned even though the update did not list it.
#[derive(Clone, Debug, Default)]
pub struct AccessibilityReader {
    nodes: BTreeMap<u64, A11yNode>,
    root: Option<u64>,
    focus: Option<u64>,
    url: String,
    title: String,
}

impl AccessibilityReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently focused node id, if any (accesskit `TreeUpdate::focus`).
    pub fn focus(&self) -> Option<u64> {
        self.focus
    }

    /// The number of live (reachable) nodes currently held.
    pub fn live_node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Apply one incremental update and return the change signal. Only the nodes
    /// in `update.nodes` are re-read from the engine; unchanged nodes are carried
    /// forward, so there is no DOM re-walk (the engine-native mutation-observer
    /// property). Bookkeeping (the live-set snapshot, reachability, and the
    /// before/after diff) is O(live nodes) per call, not O(changed nodes).
    pub fn apply_update(&mut self, update: A11yTreeUpdate) -> A11yDiff {
        let before: BTreeMap<u64, A11yNode> = self.nodes.clone();

        if let Some(root) = update.root {
            self.root = Some(root);
        }
        if let Some(focus) = update.focus {
            self.focus = Some(focus);
        }
        if let Some(url) = update.url {
            self.url = url;
        }
        if let Some(title) = update.title {
            self.title = title;
        }

        for node in update.nodes {
            self.nodes.insert(node.id, node);
        }

        // Liveness = reachable from root via children. Prune the rest (this is
        // how accesskit removals land: the parent no longer lists the child).
        let live = self.compute_reachable();
        self.nodes.retain(|id, _| live.contains(id));

        // Diff over the live set, before vs after.
        let mut added = Vec::new();
        let mut changed = Vec::new();
        for (id, node) in &self.nodes {
            match before.get(id) {
                None => added.push(*id),
                Some(prev) if prev != node => changed.push(*id),
                Some(_) => {}
            }
        }
        let removed: Vec<u64> = before
            .keys()
            .filter(|id| !self.nodes.contains_key(id))
            .copied()
            .collect();

        A11yDiff {
            added,
            changed,
            removed,
        }
    }

    /// Project the live tree into the `PageState` contract: distilled text plus
    /// interactive elements (stable id, role, name, value, bounds, visibility,
    /// degraded), in reading order.
    pub fn page_state(&self) -> PageState {
        let order = self.reading_order();
        let mut interactive_elements = Vec::new();
        let mut text_parts: Vec<String> = Vec::new();

        for id in &order {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            if let Some(text) = text_content(node) {
                text_parts.push(text);
            }
            if let Some(element) = interactive_element_for(node) {
                interactive_elements.push(element);
            }
        }

        PageState {
            url: self.url.clone(),
            title: self.title.clone(),
            distilled_text: text_parts.join(" "),
            interactive_elements,
            active_tab_id: None,
            fetch: None,
        }
    }

    /// Reading order for projection: visual (column-major) order derived from
    /// the box-tree bounds (job-008 D4, criterion 5), with document (DFS) order
    /// as the fallback for nodes the engine has not given bounds. The box tree's
    /// geometry beats DOM order on multi-column and CSS-reordered layouts.
    fn reading_order(&self) -> Vec<u64> {
        self.visual_order(self.document_order())
    }

    /// Pre-order DFS from root over `children`, skipping ids with no present
    /// node. This is document order; [`Self::reading_order`] reprojects it into
    /// visual order.
    fn document_order(&self) -> Vec<u64> {
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();
        if let Some(root) = self.root {
            self.visit(root, &mut order, &mut seen);
        } else {
            // No declared root: fall back to id order so a partial tree still
            // projects deterministically.
            order.extend(self.nodes.keys().copied());
        }
        order
    }

    /// Reproject document order into visual reading order: nodes that have bounds
    /// are grouped into columns (left-to-right) and read top-to-bottom within each
    /// column; nodes without bounds keep document order, after the positioned
    /// ones. The sort is stable, so ties preserve document order.
    fn visual_order(&self, document_order: Vec<u64>) -> Vec<u64> {
        // A typical element width is the inter-column gap threshold: a horizontal
        // jump larger than that starts a new column.
        let mut widths: Vec<f64> = document_order
            .iter()
            .filter_map(|id| self.nodes.get(id).and_then(|node| node.bounds))
            .map(|bounds| bounds.width())
            .filter(|width| *width > 0.0)
            .collect();
        let threshold = median(&mut widths).max(1.0);

        let mut x0s: Vec<f64> = document_order
            .iter()
            .filter_map(|id| self.nodes.get(id).and_then(|node| node.bounds))
            .map(|bounds| bounds.x0)
            .collect();
        x0s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        x0s.dedup();
        let boundaries = column_boundaries(&x0s, threshold);

        let mut keyed: Vec<(usize, (u8, i64, i64, i64), u64)> = document_order
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                let key = match self.nodes.get(&id).and_then(|node| node.bounds) {
                    Some(bounds) => (
                        0u8,
                        column_index(bounds.x0, &boundaries) as i64,
                        bounds.y0.round() as i64,
                        bounds.x0.round() as i64,
                    ),
                    None => (1u8, index as i64, 0, 0),
                };
                (index, key, id)
            })
            .collect();
        keyed.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        keyed.into_iter().map(|(_, _, id)| id).collect()
    }

    fn visit(&self, id: u64, order: &mut Vec<u64>, seen: &mut BTreeSet<u64>) {
        if !seen.insert(id) {
            return;
        }
        if let Some(node) = self.nodes.get(&id) {
            order.push(id);
            for child in &node.children {
                self.visit(*child, order, seen);
            }
        }
    }

    fn compute_reachable(&self) -> BTreeSet<u64> {
        let mut live = BTreeSet::new();
        let Some(root) = self.root else {
            // Without a root every present node is considered live (initial /
            // partial state); the next update with a root prunes.
            return self.nodes.keys().copied().collect();
        };
        if !self.nodes.contains_key(&root) {
            // Root declared but its node has not been supplied yet. Do NOT prune
            // to empty (that would wipe a valid partial tree); keep all present
            // nodes until the root node arrives.
            return self.nodes.keys().copied().collect();
        }
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !live.insert(id) {
                continue;
            }
            if let Some(node) = self.nodes.get(&id) {
                stack.extend(node.children.iter().copied());
            }
        }
        // Keep only ids that actually have a node present.
        live.retain(|id| self.nodes.contains_key(id));
        live
    }
}

/// Project one node into an `InteractiveElement`, or `None` if it is neither
/// interactive nor an actionable-but-unrolled container.
fn interactive_element_for(node: &A11yNode) -> Option<InteractiveElement> {
    let interactive = is_interactive_role(&node.role);
    // The engine knows it is actionable but has not given it a proper role yet
    // (D5): a GenericContainer that nonetheless has a click/default action or is
    // focusable.
    let actionable_unrolled =
        is_generic_role(&node.role) && (node.supports_default_action || node.focusable);

    if !interactive && !actionable_unrolled {
        return None;
    }

    // A disabled control is not an actionable affordance. The PageState contract
    // has no enabled/disabled slot, so rather than present a disabled element as
    // clickable (which the executor would attempt), drop it from the interactive
    // set. Its text, if any, still reaches distilled_text via text_content.
    if node.disabled {
        return None;
    }

    let bbox = node.bounds.map(|rect| rect.to_element_box());
    let has_area = node.bounds.map(|rect| rect.area() > 0.0).unwrap_or(false);
    let visible = !node.hidden && has_area;

    // Degraded when the engine has not fully rolled the node: an actionable
    // container without a proper role, or an interactive role with no bounds
    // (cannot be located precisely, prefer keyboard).
    let degraded = actionable_unrolled || (interactive && node.bounds.is_none());

    let role = normalize_role(&node.role).to_string();
    let name = node
        .label
        .clone()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| role.clone());
    let value = node.value.clone().or_else(|| {
        node.toggled
            .map(|on| if on { "true".into() } else { "false".into() })
    });

    Some(InteractiveElement {
        element_id: node.id.to_string(),
        role,
        name,
        value,
        bbox,
        visible,
        degraded,
    })
}

/// Extract the text a node contributes to the distilled reading text. accesskit
/// stores text-run / label content in `value`; other text-bearing roles carry
/// it in `label`.
fn text_content(node: &A11yNode) -> Option<String> {
    if !is_text_role(&node.role) {
        return None;
    }
    let raw = node
        .value
        .as_deref()
        .or(node.label.as_deref())
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

/// accesskit `Role` variant names that are interactive controls. Mapped to the
/// stable lowercase vocabulary the `PageState` role field uses.
fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "Link"
            | "Button"
            | "DefaultButton"
            | "DisclosureTriangle"
            | "TextInput"
            | "MultilineTextInput"
            | "SearchInput"
            | "EmailInput"
            | "UrlInput"
            | "PhoneNumberInput"
            | "NumberInput"
            | "DateInput"
            | "DateTimeInput"
            | "WeekInput"
            | "MonthInput"
            | "TimeInput"
            | "PasswordInput"
            | "CheckBox"
            | "MenuItemCheckBox"
            | "RadioButton"
            | "MenuItemRadio"
            | "Switch"
            | "ComboBox"
            | "EditableComboBox"
            | "ListBox"
            | "Slider"
            | "SpinButton"
            | "Tab"
            | "MenuItem"
            | "MenuListOption"
            | "ListBoxOption"
            | "ColorWell"
    )
}

fn is_generic_role(role: &str) -> bool {
    matches!(role, "GenericContainer" | "Group" | "Section" | "Unknown")
}

fn is_text_role(role: &str) -> bool {
    matches!(
        role,
        "TextRun" | "Label" | "StaticText" | "Paragraph" | "Heading" | "Code" | "Caption"
    )
}

/// Map an accesskit `Role` variant name to the stable lowercase role vocabulary
/// shared with the HTML reader (`link`, `button`, `textbox`, `checkbox`,
/// `radio`, `combobox`, `select`, ...). Keeps `PageState.role` consistent across
/// the two reader paths so the executor and perception layers see one vocabulary.
fn normalize_role(role: &str) -> &'static str {
    match role {
        "Link" => "link",
        "Button" | "DefaultButton" | "DisclosureTriangle" => "button",
        "TextInput" | "MultilineTextInput" | "DateInput" | "DateTimeInput" | "WeekInput"
        | "MonthInput" | "TimeInput" | "NumberInput" | "PhoneNumberInput" | "UrlInput"
        | "EmailInput" => "textbox",
        "SearchInput" => "searchbox",
        "PasswordInput" => "password",
        "CheckBox" | "MenuItemCheckBox" => "checkbox",
        "RadioButton" | "MenuItemRadio" => "radio",
        "Switch" => "switch",
        "ComboBox" | "EditableComboBox" => "combobox",
        "ListBox" => "listbox",
        "Slider" => "slider",
        "SpinButton" => "spinbutton",
        "Tab" => "tab",
        "MenuItem" => "menuitem",
        "MenuListOption" | "ListBoxOption" => "option",
        "ColorWell" => "colorpicker",
        _ => "generic",
    }
}

/// The median of a set of values (0.0 if empty). Sorts the slice in place.
fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// Column-start x0 values: the first x0, plus any x0 whose gap from the previous
/// exceeds `threshold` (the start of a new column). `sorted_x0` must be ascending
/// and deduplicated.
fn column_boundaries(sorted_x0: &[f64], threshold: f64) -> Vec<f64> {
    let mut boundaries = Vec::new();
    let mut prev: Option<f64> = None;
    for &x in sorted_x0 {
        match prev {
            None => boundaries.push(x),
            Some(p) if x - p > threshold => boundaries.push(x),
            _ => {}
        }
        prev = Some(x);
    }
    boundaries
}

/// The 0-based column index for an x0: the number of column-starts at or before
/// it, minus one (0 when there are no boundaries).
fn column_index(x0: f64, boundaries: &[f64]) -> usize {
    boundaries
        .iter()
        .filter(|&&boundary| boundary <= x0)
        .count()
        .saturating_sub(1)
}

// ===========================================================================
// D3: structured-output validation (extract(schema))
// ===========================================================================

/// The outcome of validating an extracted value against a caller schema. This
/// is Browser Use's `extract` plus its structured-output validation in one: the
/// driving model produces the candidate (the LLM-over-page-content pass), and
/// this gate decides whether it conforms before it is returned or admitted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractOutcome {
    pub url: String,
    pub value: Value,
    pub schema: Value,
    pub valid: bool,
    pub errors: Vec<String>,
}

/// Validate a model-produced candidate against the page and a caller schema.
pub fn extract_structured(page: &PageState, candidate: Value, schema: Value) -> ExtractOutcome {
    let errors = validate_against_schema(&candidate, &schema);
    ExtractOutcome {
        url: page.url.clone(),
        value: candidate,
        schema,
        valid: errors.is_empty(),
        errors,
    }
}

/// Validate a JSON value against a minimal JSON-Schema subset: `type`,
/// `required`, `properties`, `items`, and `enum`. Returns the list of violation
/// messages (empty when valid). Deliberately a focused subset, owned and tested
/// here rather than pulling a heavy validator, since the caller schemas for
/// structured extraction are simple object/array shapes.
pub fn validate_against_schema(value: &Value, schema: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_node(value, schema, "$", &mut errors);
    errors
}

fn validate_node(value: &Value, schema: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(schema_obj) = schema.as_object() else {
        // A boolean / empty schema accepts anything (JSON Schema `true`/`{}`).
        return;
    };

    if let Some(expected) = schema_obj.get("enum").and_then(Value::as_array) {
        if !expected.iter().any(|allowed| allowed == value) {
            errors.push(format!("{path}: value not in enum"));
        }
    }

    let Some(ty) = schema_obj.get("type").and_then(Value::as_str) else {
        // No declared type: required/properties still constrain.
        validate_object_members(value, schema_obj, path, errors);
        return;
    };

    if !is_known_type(ty) {
        // An unknown/misspelled type keyword must NOT silently accept anything.
        // Flag it and still check members so nested constraints are not skipped.
        errors.push(format!("{path}: unknown schema type '{ty}'"));
        validate_object_members(value, schema_obj, path, errors);
        return;
    }

    if !type_matches(value, ty) {
        errors.push(format!(
            "{path}: expected {ty}, found {}",
            json_type_name(value)
        ));
        return;
    }

    match ty {
        "object" => validate_object_members(value, schema_obj, path, errors),
        "array" => validate_array_items(value, schema_obj, path, errors),
        _ => {}
    }
}

fn validate_object_members(
    value: &Value,
    schema_obj: &serde_json::Map<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
) {
    let constrains_object =
        schema_obj.contains_key("required") || schema_obj.contains_key("properties");
    let Some(object) = value.as_object() else {
        // A schema with required/properties implies object-ness; a non-object
        // value must fail rather than slip through unchecked.
        if constrains_object {
            errors.push(format!(
                "{path}: expected object (schema declares required/properties), found {}",
                json_type_name(value)
            ));
        }
        return;
    };
    if let Some(required) = schema_obj.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                errors.push(format!("{path}.{key}: required property missing"));
            }
        }
    }
    if let Some(properties) = schema_obj.get("properties").and_then(Value::as_object) {
        for (key, sub_schema) in properties {
            if let Some(member) = object.get(key) {
                validate_node(member, sub_schema, &format!("{path}.{key}"), errors);
            }
        }
    }
}

/// Validate array elements against `items`, supporting both the uniform form
/// (`items` is one schema applied to every element) and the tuple form (`items`
/// is an array of per-position schemas). The tuple form was previously ignored,
/// letting tuple-validated arrays pass unchecked.
fn validate_array_items(
    value: &Value,
    schema_obj: &serde_json::Map<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
) {
    let Some(items) = value.as_array() else {
        return;
    };
    match schema_obj.get("items") {
        Some(Value::Array(tuple)) => {
            for (index, item) in items.iter().enumerate() {
                if let Some(sub_schema) = tuple.get(index) {
                    validate_node(item, sub_schema, &format!("{path}[{index}]"), errors);
                }
            }
        }
        Some(items_schema) => {
            for (index, item) in items.iter().enumerate() {
                validate_node(item, items_schema, &format!("{path}[{index}]"), errors);
            }
        }
        None => {}
    }
}

fn is_known_type(ty: &str) -> bool {
    matches!(
        ty,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    )
}

fn type_matches(value: &Value, ty: &str) -> bool {
    match ty {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ===========================================================================
// D3: upload path allowlist
// ===========================================================================

/// The outcome of resolving an upload path against an allowlist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UploadDecision {
    /// Permitted; carries the path to hand to the file input.
    Allowed { path: String },
    /// Refused, with the reason (no arbitrary filesystem reach).
    Refused { reason: String },
}

/// Gate a requested upload path against an allowlist of permitted root prefixes.
/// The executor only sets a file input from an `Allowed` path. An empty
/// allowlist refuses everything (upload off by default until roots are granted).
pub fn resolve_upload_path(requested: &str, allowlist: &[String]) -> UploadDecision {
    let normalized = normalize_path(requested);
    if normalized.contains("..") {
        return UploadDecision::Refused {
            reason: "path contains a parent-directory traversal".to_string(),
        };
    }
    if allowlist.is_empty() {
        return UploadDecision::Refused {
            reason: "no upload roots are allowlisted".to_string(),
        };
    }
    for root in allowlist {
        let root_norm = normalize_path(root);
        if normalized == root_norm || normalized.starts_with(&format!("{root_norm}/")) {
            return UploadDecision::Allowed { path: normalized };
        }
    }
    UploadDecision::Refused {
        reason: format!("path is not within any allowlisted root: {normalized}"),
    }
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    // Collapse redundant separators without resolving symlinks (no FS touch).
    let mut out: Vec<&str> = Vec::new();
    for part in trimmed.split('/') {
        match part {
            "" | "." => {}
            other => out.push(other),
        }
    }
    let prefix = if trimmed.starts_with('/') { "/" } else { "" };
    // Preserve a single ".." token so the traversal check above can reject it.
    if trimmed.contains("..") {
        return format!(
            "{prefix}{}",
            trimmed
                .split('/')
                .filter(|p| !p.is_empty() && *p != ".")
                .collect::<Vec<_>>()
                .join("/")
        );
    }
    format!("{prefix}{}", out.join("/"))
}

// ===========================================================================
// D4: domain restriction
// ===========================================================================

/// The set of domains a session may navigate or act within. An empty permitted
/// set means no restriction is configured (open browsing); a non-empty set is an
/// allowlist and anything outside it is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainPolicy {
    #[serde(default)]
    pub permitted: Vec<String>,
}

/// The decision for a navigation or action target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NavigationDecision {
    Permitted,
    Refused { reason: String },
}

impl DomainPolicy {
    pub fn new(permitted: Vec<String>) -> Self {
        Self { permitted }
    }

    /// The non-blank permitted entries. Blank/whitespace entries are ignored so a
    /// stray `""` cannot silently brick all navigation.
    fn effective_permitted(&self) -> Vec<&str> {
        self.permitted
            .iter()
            .map(|d| d.trim())
            .filter(|d| !d.is_empty())
            .collect()
    }

    /// No effective restriction configured (open browsing). An allowlist that is
    /// empty, or that contains only blank entries, is unrestricted.
    pub fn is_unrestricted(&self) -> bool {
        self.effective_permitted().is_empty()
    }

    /// Evaluate a target URL against the policy.
    pub fn evaluate(&self, url: &str) -> NavigationDecision {
        let permitted = self.effective_permitted();
        if permitted.is_empty() {
            return NavigationDecision::Permitted;
        }
        let host = match Url::parse(url) {
            Ok(parsed) => parsed.host_str().map(normalize_host),
            Err(_) => None,
        };
        let Some(host) = host else {
            return NavigationDecision::Refused {
                reason: format!("target has no resolvable host: {url}"),
            };
        };
        if permitted.iter().any(|domain| host_matches(&host, domain)) {
            NavigationDecision::Permitted
        } else {
            NavigationDecision::Refused {
                reason: format!("{host} is outside the permitted domain set"),
            }
        }
    }
}

/// Normalise a host for comparison: lowercase and strip a trailing FQDN dot, so
/// the absolute form `example.com.` compares equal to `example.com`.
fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// A host matches a permitted domain if it equals it or is a subdomain of it.
/// Both sides are trailing-dot normalised so the absolute-FQDN form cannot be
/// used to make an allowlisted host read as off-domain.
fn host_matches(host: &str, domain: &str) -> bool {
    let domain = domain
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    let host = host.trim_end_matches('.');
    host == domain || host.ends_with(&format!(".{domain}"))
}

// ===========================================================================
// D4: sensitive-data masking
// ===========================================================================

/// Marker substituted into any logged text in place of a secret value. The
/// literal secret never enters trace events, model context, or receipts; the
/// trace shows this marker instead.
fn secret_marker(domain: &str, key: &str) -> String {
    format!("<secret:{domain}/{key}>")
}

/// Domain-scoped secrets. Values are substituted into `Type`/upload at the
/// engine boundary by the executor (via [`SensitiveData::resolve`]), while every
/// string that is logged is first passed through [`SensitiveData::mask`] so the
/// literal value never escapes. A `"*"` domain entry applies to all domains.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensitiveData {
    #[serde(default)]
    domain_scoped: BTreeMap<String, BTreeMap<String, String>>,
}

/// The result of masking a string: the masked text plus which keys were hit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedText {
    pub masked: String,
    pub used_keys: Vec<String>,
}

impl SensitiveData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a secret for a domain (use `"*"` for all domains).
    pub fn set(
        &mut self,
        domain: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.domain_scoped
            .entry(domain.into())
            .or_default()
            .insert(key.into(), value.into());
    }

    /// The real secret value for a domain/key, for the executor to substitute
    /// at the engine boundary. Falls back to a `"*"` (all-domain) entry.
    pub fn resolve(&self, domain: &str, key: &str) -> Option<&str> {
        self.domain_scoped
            .get(domain)
            .and_then(|m| m.get(key))
            .or_else(|| self.domain_scoped.get("*").and_then(|m| m.get(key)))
            .map(String::as_str)
    }

    /// Replace a `{{secret:key}}` placeholder with the real value for the
    /// executor. Unknown keys are left intact. This is the only place a literal
    /// secret is produced, and only for the engine boundary.
    pub fn resolve_placeholders(&self, domain: &str, text: &str) -> String {
        let mut out = text.to_string();
        for (key, value) in self.entries_for(domain) {
            out = out.replace(&format!("{{{{secret:{key}}}}}"), &value);
        }
        out
    }

    /// Mask any occurrence of a known secret value (for `domain`) in arbitrary
    /// text, replacing it with the [`secret_marker`]. Use on everything logged:
    /// trace, model context, receipts.
    ///
    /// Leak-safety, three properties the naive `replace` loop did not have:
    /// 1. Longest value first, so a short secret that is a prefix/substring of a
    ///    longer one cannot mask first and leave the longer one's tail in the text.
    /// 2. The union of all applicable secrets (both the `"*"` entry and the
    ///    domain entry, even when they share a key), so a wildcard secret is never
    ///    shadowed out of masking by a same-key domain secret.
    /// 3. A single forward scan (a tokenizer, not find-and-replace): markers are
    ///    written to a fresh output buffer that is never re-scanned, so a secret
    ///    value that collides with the marker template (e.g. a secret literally
    ///    equal to "secret") can never rewrite an already-emitted marker.
    pub fn mask(&self, domain: &str, text: &str) -> MaskedText {
        let mut secrets = self.applicable_secrets(domain);
        secrets.retain(|secret| !secret.value.is_empty());
        // Longest value first; ties broken by marker for determinism. Longest
        // first means a short secret that is a prefix of a longer one cannot
        // match first and leave the longer one's tail behind.
        secrets.sort_by(|a, b| {
            b.value
                .len()
                .cmp(&a.value.len())
                .then_with(|| a.marker.cmp(&b.marker))
        });

        let mut masked = String::with_capacity(text.len());
        let mut used_keys = Vec::new();
        let mut rest = text;
        'scan: while !rest.is_empty() {
            for secret in &secrets {
                if let Some(stripped) = rest.strip_prefix(secret.value.as_str()) {
                    masked.push_str(&secret.marker);
                    used_keys.push(secret.key.clone());
                    rest = stripped;
                    continue 'scan;
                }
            }
            // No secret matches at the front: copy one char (UTF-8 safe) and advance.
            let mut chars = rest.chars();
            let ch = chars.next().expect("rest is non-empty");
            masked.push(ch);
            rest = chars.as_str();
        }
        used_keys.sort();
        used_keys.dedup();
        MaskedText { masked, used_keys }
    }

    /// Every secret that applies to `domain`: the `"*"` (all-domain) entries and
    /// the domain's own entries, kept distinct (a `"*"` and a domain secret with
    /// the same key are two different secrets and both must be masked).
    fn applicable_secrets(&self, domain: &str) -> Vec<ApplicableSecret> {
        let mut out = Vec::new();
        if let Some(global) = self.domain_scoped.get("*") {
            for (key, value) in global {
                out.push(ApplicableSecret {
                    key: key.clone(),
                    marker: secret_marker("*", key),
                    value: value.clone(),
                });
            }
        }
        if domain != "*" {
            if let Some(scoped) = self.domain_scoped.get(domain) {
                for (key, value) in scoped {
                    out.push(ApplicableSecret {
                        key: key.clone(),
                        marker: secret_marker(domain, key),
                        value: value.clone(),
                    });
                }
            }
        }
        out
    }

    /// All (key, value) pairs that apply to a domain (its own plus `"*"`), with
    /// the domain entry shadowing `"*"` on a shared key. Used by
    /// [`Self::resolve_placeholders`], where keys are uniquely delimited so the
    /// shadow is the intended fallback semantics.
    fn entries_for(&self, domain: &str) -> Vec<(String, String)> {
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        if let Some(global) = self.domain_scoped.get("*") {
            for (k, v) in global {
                pairs.insert(k.clone(), v.clone());
            }
        }
        if let Some(scoped) = self.domain_scoped.get(domain) {
            for (k, v) in scoped {
                pairs.insert(k.clone(), v.clone());
            }
        }
        pairs.into_iter().collect()
    }
}

/// One secret resolved for masking: its key (for `used_keys`), its display
/// marker, and the literal value to redact.
struct ApplicableSecret {
    key: String,
    marker: String,
    value: String,
}

// ===========================================================================
// D4: download detection
// ===========================================================================

/// Metadata for a download triggered by an action (e.g. a click that resolves
/// to a file rather than a navigable document).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadMeta {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

/// The response signal the engine/embedder reports for a load resulting from an
/// action, enough to recognise a download.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseSignal {
    pub final_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_disposition: Option<String>,
}

/// Detect whether an action's resulting response is a download. A response is a
/// download when it carries `Content-Disposition: attachment` (the
/// authoritative signal) or a non-renderable octet-stream content type. Returns
/// the metadata to place in the `observe()` delta, or `None` for a normal load.
pub fn detect_download(signal: &ResponseSignal) -> Option<DownloadMeta> {
    let disposition = signal
        .content_disposition
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let content_type = signal
        .content_type
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let is_attachment = disposition.contains("attachment");
    let is_octet_stream = content_type.starts_with("application/octet-stream");

    if !is_attachment && !is_octet_stream {
        return None;
    }

    Some(DownloadMeta {
        url: signal.final_url.clone(),
        suggested_filename: filename_from_disposition(&disposition)
            .or_else(|| filename_from_url(&signal.final_url)),
        mime: signal.content_type.clone(),
    })
}

fn filename_from_disposition(disposition: &str) -> Option<String> {
    let marker = "filename=";
    let start = disposition.find(marker)? + marker.len();
    let rest = disposition[start..].trim();
    let value = rest
        .trim_start_matches('"')
        .split(['"', ';'])
        .next()
        .unwrap_or("")
        .trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn filename_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let last = parsed.path_segments()?.next_back()?;
    if last.is_empty() {
        None
    } else {
        Some(last.to_string())
    }
}

// ===========================================================================
// D3: the tab model
// ===========================================================================

/// One tab in the session, modelled at the agent layer. The binding to a Servo
/// webview is the executor/embedder seam; this is the state the agent reasons
/// over and the `PageState` active-tab tracking rides on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub active: bool,
}

/// The set of open tabs with the agent-layer open/switch/close/list operations.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabSet {
    tabs: Vec<Tab>,
    next_id: u64,
}

impl TabSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new tab for `url`, make it active, and return its id.
    pub fn open(&mut self, url: impl Into<String>) -> String {
        let id = format!("tab-{}", self.next_id);
        self.next_id += 1;
        for tab in &mut self.tabs {
            tab.active = false;
        }
        self.tabs.push(Tab {
            id: id.clone(),
            url: url.into(),
            title: String::new(),
            active: true,
        });
        id
    }

    /// Switch the active tab to `id`.
    pub fn switch(&mut self, id: &str) -> Result<(), String> {
        if !self.tabs.iter().any(|tab| tab.id == id) {
            return Err(format!("no such tab: {id}"));
        }
        for tab in &mut self.tabs {
            tab.active = tab.id == id;
        }
        Ok(())
    }

    /// Close `id`. If it was active, the last remaining tab becomes active.
    pub fn close(&mut self, id: &str) -> Result<(), String> {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return Err(format!("no such tab: {id}"));
        };
        let was_active = self.tabs[index].active;
        self.tabs.remove(index);
        if was_active {
            if let Some(last) = self.tabs.last_mut() {
                last.active = true;
            }
        }
        Ok(())
    }

    pub fn list(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.active)
    }

    pub fn update_active(
        &mut self,
        url: impl Into<String>,
        title: impl Into<String>,
    ) -> Option<String> {
        let tab = self.tabs.iter_mut().find(|tab| tab.active)?;
        tab.url = url.into();
        tab.title = title.into();
        Some(tab.id.clone())
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

// ===========================================================================
// D5: keyboard fallback for degraded elements
// ===========================================================================

/// A keyboard-activation plan the executor runs (via SendKeys) when an element
/// is degraded and a precise click is unsafe. The element is reached by focus
/// traversal first; `keys` are the activation keys once focused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardFallback {
    pub element_id: String,
    pub keys: Vec<String>,
    pub note: String,
}

/// The keyboard fallback for an element, or `None` if a normal click is fine.
/// Degraded elements (D5) and elements without bounds get a keyboard plan so
/// the loop stays operable before the engine rolls full interactivity.
pub fn keyboard_fallback_for(element: &InteractiveElement) -> Option<KeyboardFallback> {
    if !element.degraded && element.bbox.is_some() {
        return None;
    }
    let keys = match element.role.as_str() {
        "checkbox" | "switch" | "radio" => vec!["Space".to_string()],
        "textbox" | "searchbox" | "password" | "combobox" => vec!["Enter".to_string()],
        _ => vec!["Enter".to_string()],
    };
    Some(KeyboardFallback {
        element_id: element.element_id.clone(),
        keys,
        note: "focus via Tab traversal, then activate".to_string(),
    })
}

// ===========================================================================
// The real accesskit boundary (optional `accesskit` feature)
//
// This is the conversion the Servo embedder calls: it maps a live
// `accesskit::TreeUpdate` (what Servo emits) into the [`A11yTreeUpdate`] DTO the
// reader consumes. It lives behind a feature so the default agent build carries
// no accesskit pin; the embedder enables `accesskit` and feeds Servo's tree.
// ===========================================================================

#[cfg(feature = "accesskit")]
mod accesskit_bridge {
    use super::{A11yNode, A11yRect, A11yTreeUpdate};
    use accesskit::{Action, Node, NodeId, Rect, Toggled, TreeUpdate};

    impl A11yRect {
        /// Project an `accesskit::Rect` into the DTO rectangle.
        pub fn from_accesskit(rect: &Rect) -> Self {
            Self {
                x0: rect.x0,
                y0: rect.y0,
                x1: rect.x1,
                y1: rect.y1,
            }
        }
    }

    /// Map one `accesskit::Node` (with its id) into the DTO node, using only
    /// stable accessors. The role is the `Debug` variant name (e.g. `"Button"`),
    /// which is exactly what [`super::normalize_role`] / [`super::is_interactive_role`]
    /// match on, so the agent layer never enumerates accesskit's roles.
    pub fn a11y_node_from_accesskit(id: NodeId, node: &Node) -> A11yNode {
        A11yNode {
            id: id.0,
            role: format!("{:?}", node.role()),
            label: node.label().map(str::to_string),
            value: node.value().map(str::to_string),
            bounds: node.bounds().map(|rect| A11yRect::from_accesskit(&rect)),
            hidden: node.is_hidden(),
            disabled: node.is_disabled(),
            focusable: node.supports_action(Action::Focus),
            supports_default_action: node.supports_action(Action::Click),
            toggled: match node.toggled() {
                Some(Toggled::True) => Some(true),
                Some(Toggled::False) => Some(false),
                Some(Toggled::Mixed) | None => None,
            },
            children: node.children().iter().map(|child| child.0).collect(),
        }
    }

    impl A11yTreeUpdate {
        /// Build the DTO update from a live `accesskit::TreeUpdate`. `url`/`title`
        /// come from the embedder (which knows the loaded document); they are not
        /// accesskit tree properties.
        pub fn from_accesskit(
            update: &TreeUpdate,
            url: Option<String>,
            title: Option<String>,
        ) -> Self {
            Self {
                nodes: update
                    .nodes
                    .iter()
                    .map(|(id, node)| a11y_node_from_accesskit(*id, node))
                    .collect(),
                root: update.tree.as_ref().map(|tree| tree.root.0),
                focus: Some(update.focus.0),
                url,
                title,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: u64, role: &str) -> A11yNode {
        A11yNode {
            id,
            role: role.to_string(),
            ..A11yNode::default()
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> A11yRect {
        A11yRect { x0, y0, x1, y1 }
    }

    /// A small form page: root -> [heading, link, button, text input].
    fn form_tree() -> A11yTreeUpdate {
        let mut heading = node(2, "Heading");
        heading.value = Some("Sign in".to_string());

        let mut link = node(3, "Link");
        link.label = Some("Help".to_string());
        link.bounds = Some(rect(0.0, 40.0, 60.0, 60.0));

        let mut button = node(4, "Button");
        button.label = Some("Save".to_string());
        button.bounds = Some(rect(0.0, 70.0, 80.0, 100.0));

        let mut input = node(5, "TextInput");
        input.label = Some("Query".to_string());
        input.value = Some("rust".to_string());
        input.bounds = Some(rect(0.0, 110.0, 200.0, 140.0));

        let mut root = node(1, "RootWebArea");
        root.children = vec![2, 3, 4, 5];

        A11yTreeUpdate {
            nodes: vec![root, heading, link, button, input],
            root: Some(1),
            focus: Some(5),
            url: Some("https://example.com/login".to_string()),
            title: Some("Login".to_string()),
        }
    }

    // ---- D1 / acceptance criterion 1 --------------------------------------

    #[test]
    fn reader_lists_controls_with_stable_ids_names_values_and_bounds() {
        let mut reader = AccessibilityReader::new();
        reader.apply_update(form_tree());
        let page = reader.page_state();

        assert_eq!(page.url, "https://example.com/login");
        assert_eq!(page.title, "Login");
        assert!(page.distilled_text.contains("Sign in"));

        // link, button, text input -> three interactive elements (heading is not).
        assert_eq!(page.interactive_elements.len(), 3);

        let button = page
            .interactive_elements
            .iter()
            .find(|el| el.role == "button")
            .expect("button present");
        assert_eq!(button.element_id, "4"); // the accesskit NodeId, not "e{index}"
        assert_eq!(button.name, "Save");
        let bbox = button.bbox.as_ref().expect("button has bounds");
        assert_eq!(bbox.width, 80);
        assert_eq!(bbox.height, 30);
        assert!(button.visible);
        assert!(!button.degraded);

        let input = page
            .interactive_elements
            .iter()
            .find(|el| el.role == "textbox")
            .expect("textbox present");
        assert_eq!(input.element_id, "5");
        assert_eq!(input.name, "Query");
        assert_eq!(input.value.as_deref(), Some("rust"));
    }

    // ---- D1 / acceptance criterion 2 --------------------------------------

    #[test]
    fn incremental_update_changes_one_node_without_a_full_rewalk() {
        let mut reader = AccessibilityReader::new();
        reader.apply_update(form_tree());

        // Change only the text input's value; supply just that one node.
        let mut changed_input = node(5, "TextInput");
        changed_input.label = Some("Query".to_string());
        changed_input.value = Some("servo".to_string());
        changed_input.bounds = Some(rect(0.0, 110.0, 200.0, 140.0));

        let diff = reader.apply_update(A11yTreeUpdate {
            nodes: vec![changed_input],
            focus: Some(5),
            ..A11yTreeUpdate::default()
        });

        assert_eq!(diff.changed, vec![5]);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());

        let page = reader.page_state();
        // The other three nodes survived without being re-supplied.
        assert_eq!(page.interactive_elements.len(), 3);
        let input = page
            .interactive_elements
            .iter()
            .find(|el| el.element_id == "5")
            .unwrap();
        assert_eq!(input.value.as_deref(), Some("servo"));
    }

    #[test]
    fn removing_a_child_from_its_parent_prunes_the_subtree() {
        let mut reader = AccessibilityReader::new();
        reader.apply_update(form_tree());
        assert_eq!(reader.live_node_count(), 5);

        // Re-send the root without the button (id 4) in children: accesskit's
        // removal contract. The button is not in the update node list.
        let mut root = node(1, "RootWebArea");
        root.children = vec![2, 3, 5];
        let diff = reader.apply_update(A11yTreeUpdate {
            nodes: vec![root],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });

        assert_eq!(diff.removed, vec![4]);
        assert_eq!(reader.live_node_count(), 4);
        let page = reader.page_state();
        assert!(page
            .interactive_elements
            .iter()
            .all(|el| el.element_id != "4"));
    }

    // ---- D5 / acceptance criterion 10 -------------------------------------

    #[test]
    fn unrolled_actionable_container_is_marked_degraded_and_has_keyboard_fallback() {
        let mut container = node(2, "GenericContainer");
        container.supports_default_action = true;
        container.label = Some("Fancy widget".to_string());
        // No bounds: the engine has not rolled it.

        let mut root = node(1, "RootWebArea");
        root.children = vec![2];

        let mut reader = AccessibilityReader::new();
        reader.apply_update(A11yTreeUpdate {
            nodes: vec![root, container],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });

        let page = reader.page_state();
        let widget = page
            .interactive_elements
            .iter()
            .find(|el| el.element_id == "2")
            .expect("degraded widget surfaced, not dropped");
        assert!(widget.degraded);
        assert!(!widget.visible); // no bounds -> not precisely clickable

        let fallback = keyboard_fallback_for(widget).expect("degraded element has a keyboard plan");
        assert_eq!(fallback.keys, vec!["Enter".to_string()]);
    }

    #[test]
    fn interactive_role_without_bounds_is_degraded() {
        let mut button = node(2, "Button");
        button.label = Some("Ghost".to_string());
        let mut root = node(1, "RootWebArea");
        root.children = vec![2];
        let mut reader = AccessibilityReader::new();
        reader.apply_update(A11yTreeUpdate {
            nodes: vec![root, button],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });
        let page = reader.page_state();
        let el = &page.interactive_elements[0];
        assert!(el.degraded);
    }

    // ---- D4 reading order / acceptance criterion 5 ------------------------

    #[test]
    fn reading_order_is_visual_column_major_not_document_order() {
        // Two columns. Document/tree order interleaves them (c1r1, c2r1, c1r2,
        // c2r2), but visual reading order is column-major: down column one, then
        // down column two.
        let mut c1r1 = node(2, "Link");
        c1r1.label = Some("c1r1".to_string());
        c1r1.bounds = Some(rect(0.0, 0.0, 80.0, 20.0));
        let mut c2r1 = node(3, "Link");
        c2r1.label = Some("c2r1".to_string());
        c2r1.bounds = Some(rect(400.0, 0.0, 480.0, 20.0));
        let mut c1r2 = node(4, "Link");
        c1r2.label = Some("c1r2".to_string());
        c1r2.bounds = Some(rect(0.0, 40.0, 80.0, 60.0));
        let mut c2r2 = node(5, "Link");
        c2r2.label = Some("c2r2".to_string());
        c2r2.bounds = Some(rect(400.0, 40.0, 480.0, 60.0));

        let mut root = node(1, "RootWebArea");
        root.children = vec![2, 3, 4, 5]; // interleaved across columns

        let mut reader = AccessibilityReader::new();
        reader.apply_update(A11yTreeUpdate {
            nodes: vec![root, c1r1, c2r1, c1r2, c2r2],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });
        let page = reader.page_state();
        let names: Vec<&str> = page
            .interactive_elements
            .iter()
            .map(|el| el.name.as_str())
            .collect();
        assert_eq!(names, vec!["c1r1", "c1r2", "c2r1", "c2r2"]);
    }

    #[test]
    fn single_column_reading_order_is_top_to_bottom() {
        // All in one column: visual order is by y, regardless of tree order.
        let mut low = node(2, "Link");
        low.label = Some("low".to_string());
        low.bounds = Some(rect(0.0, 100.0, 80.0, 120.0));
        let mut high = node(3, "Link");
        high.label = Some("high".to_string());
        high.bounds = Some(rect(0.0, 0.0, 80.0, 20.0));
        let mut root = node(1, "RootWebArea");
        root.children = vec![2, 3]; // tree order: low before high

        let mut reader = AccessibilityReader::new();
        reader.apply_update(A11yTreeUpdate {
            nodes: vec![root, low, high],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });
        let page = reader.page_state();
        let names: Vec<&str> = page
            .interactive_elements
            .iter()
            .map(|el| el.name.as_str())
            .collect();
        assert_eq!(names, vec!["high", "low"]); // visual top-to-bottom
    }

    // ---- D3 extract(schema) / acceptance criterion 4 ----------------------

    #[test]
    fn extract_validates_a_conforming_value_against_schema() {
        let page = PageState {
            url: "https://example.com".into(),
            title: "T".into(),
            distilled_text: String::new(),
            interactive_elements: Vec::new(),
            active_tab_id: None,
            fetch: None,
        };
        let schema = json!({
            "type": "object",
            "required": ["title", "items"],
            "properties": {
                "title": { "type": "string" },
                "items": {
                    "type": "array",
                    "items": { "type": "object", "required": ["name"], "properties": { "name": { "type": "string" } } }
                }
            }
        });
        let good = json!({ "title": "Docs", "items": [ { "name": "a" }, { "name": "b" } ] });
        let outcome = extract_structured(&page, good, schema.clone());
        assert!(outcome.valid, "errors: {:?}", outcome.errors);

        let bad = json!({ "title": 7, "items": [ { "nope": "a" } ] });
        let outcome = extract_structured(&page, bad, schema);
        assert!(!outcome.valid);
        assert!(outcome.errors.iter().any(|e| e.contains("title")));
        assert!(outcome.errors.iter().any(|e| e.contains("name")));
    }

    #[test]
    fn schema_enum_is_enforced() {
        let schema = json!({ "type": "string", "enum": ["a", "b"] });
        assert!(validate_against_schema(&json!("a"), &schema).is_empty());
        assert!(!validate_against_schema(&json!("c"), &schema).is_empty());
    }

    // ---- D3 upload allowlist / acceptance criterion 5 ---------------------

    #[test]
    fn upload_allows_inside_root_and_refuses_outside_or_traversal() {
        let allow = vec!["/var/uploads".to_string()];
        assert_eq!(
            resolve_upload_path("/var/uploads/report.pdf", &allow),
            UploadDecision::Allowed {
                path: "/var/uploads/report.pdf".into()
            }
        );
        assert!(matches!(
            resolve_upload_path("/etc/passwd", &allow),
            UploadDecision::Refused { .. }
        ));
        assert!(matches!(
            resolve_upload_path("/var/uploads/../../etc/passwd", &allow),
            UploadDecision::Refused { .. }
        ));
        assert!(matches!(
            resolve_upload_path("/var/uploads/x", &[]),
            UploadDecision::Refused { .. }
        ));
    }

    // ---- D3 tabs / acceptance criterion 6 ---------------------------------

    #[test]
    fn tab_set_open_switch_close_tracks_active() {
        let mut tabs = TabSet::new();
        let a = tabs.open("https://a.com");
        let b = tabs.open("https://b.com");
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active().unwrap().id, b);

        tabs.switch(&a).unwrap();
        assert_eq!(tabs.active().unwrap().id, a);

        tabs.close(&a).unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active().unwrap().id, b); // closing active reactivates another
        assert!(tabs.switch("tab-999").is_err());
    }

    // ---- D4 sensitive data / acceptance criterion 7 -----------------------

    #[test]
    fn sensitive_value_is_masked_everywhere_but_resolvable_for_the_executor() {
        let mut secrets = SensitiveData::new();
        secrets.set("example.com", "password", "hunter2");

        // The executor can resolve the real value at the engine boundary.
        assert_eq!(secrets.resolve("example.com", "password"), Some("hunter2"));
        assert_eq!(
            secrets.resolve_placeholders("example.com", "login with {{secret:password}}"),
            "login with hunter2"
        );

        // Everything logged is masked: the literal never appears, the marker does.
        let masked = secrets.mask("example.com", "typed hunter2 into the field");
        assert!(!masked.masked.contains("hunter2"));
        assert!(masked.masked.contains("<secret:example.com/password>"));
        assert_eq!(masked.used_keys, vec!["password".to_string()]);

        // Domain scoping: another domain does not leak this secret.
        assert_eq!(secrets.resolve("other.com", "password"), None);
        let other = secrets.mask("other.com", "typed hunter2 into the field");
        assert!(other.masked.contains("hunter2")); // not masked off-domain (caller scopes by domain)
    }

    // ---- D4 domain restriction / acceptance criterion 8 -------------------

    #[test]
    fn domain_policy_permits_allowlisted_and_subdomains_refuses_others() {
        let policy = DomainPolicy::new(vec!["example.com".to_string()]);
        assert_eq!(
            policy.evaluate("https://example.com/x"),
            NavigationDecision::Permitted
        );
        assert_eq!(
            policy.evaluate("https://docs.example.com/y"),
            NavigationDecision::Permitted
        );
        assert!(matches!(
            policy.evaluate("https://evil.com/z"),
            NavigationDecision::Refused { .. }
        ));
        // empty allowlist = unrestricted
        assert_eq!(
            DomainPolicy::default().evaluate("https://anything.com"),
            NavigationDecision::Permitted
        );
        // a near-miss suffix must not match (notexample.com vs example.com)
        assert!(matches!(
            policy.evaluate("https://notexample.com"),
            NavigationDecision::Refused { .. }
        ));
    }

    // ---- D4 download detection --------------------------------------------

    #[test]
    fn download_detected_from_attachment_disposition() {
        let signal = ResponseSignal {
            final_url: "https://example.com/files/report.pdf".into(),
            content_type: Some("application/pdf".into()),
            content_disposition: Some("attachment; filename=\"report.pdf\"".into()),
        };
        let meta = detect_download(&signal).expect("download detected");
        assert_eq!(meta.suggested_filename.as_deref(), Some("report.pdf"));
        assert_eq!(meta.mime.as_deref(), Some("application/pdf"));

        // A normal HTML navigation is not a download.
        let nav = ResponseSignal {
            final_url: "https://example.com/page".into(),
            content_type: Some("text/html".into()),
            content_disposition: None,
        };
        assert!(detect_download(&nav).is_none());
    }

    #[test]
    fn download_detected_from_octet_stream() {
        let signal = ResponseSignal {
            final_url: "https://example.com/d/blob.bin".into(),
            content_type: Some("application/octet-stream".into()),
            content_disposition: None,
        };
        let meta = detect_download(&signal).expect("octet-stream is a download");
        assert_eq!(meta.suggested_filename.as_deref(), Some("blob.bin"));
    }

    // ---- adversarial regressions (holes found by the acceptance review) ----

    #[test]
    fn masking_does_not_leak_when_one_secret_is_a_substring_of_another() {
        // 'pass'='abc' is a prefix of 'token'='abc123'. Naive key-order masking
        // would replace 'abc' first and leave '123' (the tail of the real token)
        // in the trace. Longest-first masking must redact both fully.
        let mut secrets = SensitiveData::new();
        secrets.set("ex.com", "pass", "abc");
        secrets.set("ex.com", "token", "abc123");
        let masked = secrets.mask("ex.com", "value abc123 here");
        assert!(!masked.masked.contains("abc123"));
        assert!(!masked.masked.contains("123"));
        assert!(masked.masked.contains("<secret:ex.com/token>"));
    }

    #[test]
    fn masking_redacts_a_wildcard_secret_even_when_a_domain_shares_the_key() {
        // A '*' (all-domain) secret and a domain secret share key 'token' but hold
        // different values; both must be masked (the domain value must not shadow
        // the wildcard value out of masking).
        let mut secrets = SensitiveData::new();
        secrets.set("*", "token", "GLOBAL_SECRET");
        secrets.set("ex.com", "token", "DOMAIN_SECRET");
        let masked = secrets.mask("ex.com", "leak GLOBAL_SECRET and DOMAIN_SECRET");
        assert!(!masked.masked.contains("GLOBAL_SECRET"));
        assert!(!masked.masked.contains("DOMAIN_SECRET"));
    }

    #[test]
    fn masking_is_not_corrupted_by_a_secret_that_collides_with_the_marker() {
        // A secret literally equal to 'secret' appears inside every marker. The
        // two-phase sentinel substitution must keep it from rewriting an already
        // inserted marker.
        let mut secrets = SensitiveData::new();
        secrets.set("ex.com", "a", "TOPSECRET");
        secrets.set("ex.com", "b", "secret");
        let masked = secrets.mask("ex.com", "reveal TOPSECRET now");
        assert!(!masked.masked.contains("TOPSECRET"));
        assert!(masked.masked.contains("<secret:ex.com/a>"));
        // 'b' never matched the user text, so it must not be reported used.
        assert_eq!(masked.used_keys, vec!["a".to_string()]);
    }

    #[test]
    fn schema_unknown_type_keyword_does_not_accept_anything() {
        let schema = json!({ "type": "frobnicate", "required": ["mustExist"] });
        let errors = validate_against_schema(&json!({ "whatever": [1, 2, 3] }), &schema);
        assert!(errors.iter().any(|e| e.contains("unknown schema type")));
        assert!(errors.iter().any(|e| e.contains("mustExist")));
    }

    #[test]
    fn schema_required_without_type_rejects_a_non_object() {
        let schema = json!({ "required": ["mustExist"] });
        let errors = validate_against_schema(&json!("a string"), &schema);
        assert!(
            !errors.is_empty(),
            "non-object must fail a required-bearing schema"
        );
    }

    #[test]
    fn schema_tuple_form_items_are_validated_positionally() {
        let schema =
            json!({ "type": "array", "items": [ { "type": "integer" }, { "type": "string" } ] });
        assert!(validate_against_schema(&json!([1, "two"]), &schema).is_empty());
        let errors = validate_against_schema(&json!(["one", "two"]), &schema);
        assert!(errors.iter().any(|e| e.contains("[0]")));
    }

    #[test]
    fn domain_trailing_dot_fqdn_matches_both_directions() {
        // Absolute-FQDN host must not read as off-domain against a bare allowlist.
        let policy = DomainPolicy::new(vec!["example.com".to_string()]);
        assert_eq!(
            policy.evaluate("https://example.com./x"),
            NavigationDecision::Permitted
        );
        // And a dotted allowlist entry must still match the bare host.
        let dotted = DomainPolicy::new(vec!["example.com.".to_string()]);
        assert_eq!(
            dotted.evaluate("https://example.com/x"),
            NavigationDecision::Permitted
        );
        assert_eq!(
            dotted.evaluate("https://sub.example.com/x"),
            NavigationDecision::Permitted
        );
    }

    #[test]
    fn domain_blank_allowlist_entries_do_not_brick_navigation() {
        // An all-blank allowlist reads as unrestricted, not "refuse everything".
        let all_blank = DomainPolicy::new(vec!["   ".to_string(), "".to_string()]);
        assert!(all_blank.is_unrestricted());
        assert_eq!(
            all_blank.evaluate("https://anything.com"),
            NavigationDecision::Permitted
        );
        // A real entry alongside a blank still enforces the real one.
        let mixed = DomainPolicy::new(vec!["example.com".to_string(), "".to_string()]);
        assert!(!mixed.is_unrestricted());
        assert!(matches!(
            mixed.evaluate("https://evil.com"),
            NavigationDecision::Refused { .. }
        ));
    }

    #[test]
    fn disabled_controls_are_not_surfaced_as_interactive() {
        let mut disabled = node(2, "Button");
        disabled.label = Some("Submit".to_string());
        disabled.disabled = true;
        disabled.bounds = Some(rect(0.0, 0.0, 50.0, 20.0));
        let mut root = node(1, "RootWebArea");
        root.children = vec![2];
        let mut reader = AccessibilityReader::new();
        reader.apply_update(A11yTreeUpdate {
            nodes: vec![root, disabled],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });
        let page = reader.page_state();
        assert!(page.interactive_elements.is_empty());
    }

    #[test]
    fn root_declared_without_its_node_does_not_wipe_the_tree() {
        let mut reader = AccessibilityReader::new();
        // Supply two child nodes and declare root id 1, but never supply node 1.
        reader.apply_update(A11yTreeUpdate {
            nodes: vec![node(2, "Button"), node(3, "Link")],
            root: Some(1),
            ..A11yTreeUpdate::default()
        });
        // The partial tree survives rather than being pruned to empty.
        assert_eq!(reader.live_node_count(), 2);
    }
}

#[cfg(all(test, feature = "accesskit"))]
mod accesskit_tests {
    use super::*;
    use accesskit::{Node, NodeId, Rect, Role, Tree, TreeId, TreeUpdate};

    #[test]
    fn real_accesskit_tree_converts_and_reads_through_the_reader() {
        let mut button = Node::new(Role::Button);
        button.set_label("Save");
        button.set_bounds(Rect {
            x0: 0.0,
            y0: 70.0,
            x1: 80.0,
            y1: 100.0,
        });

        let mut root = Node::new(Role::RootWebArea);
        root.set_children(vec![NodeId(2)]);

        let update = TreeUpdate {
            nodes: vec![(NodeId(1), root), (NodeId(2), button)],
            tree: Some(Tree::new(NodeId(1))),
            tree_id: TreeId::ROOT,
            focus: NodeId(2),
        };

        let dto = A11yTreeUpdate::from_accesskit(
            &update,
            Some("https://example.com/login".to_string()),
            Some("Login".to_string()),
        );
        assert_eq!(dto.root, Some(1));
        assert_eq!(dto.focus, Some(2));

        let mut reader = AccessibilityReader::new();
        reader.apply_update(dto);
        let page = reader.page_state();
        let button = page
            .interactive_elements
            .iter()
            .find(|el| el.element_id == "2")
            .expect("button converted from accesskit");
        assert_eq!(button.role, "button");
        assert_eq!(button.name, "Save");
        assert_eq!(button.bbox.as_ref().unwrap().width, 80);
        assert!(!button.degraded);
    }
}
