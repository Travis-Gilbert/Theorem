//! Servo browser-use, build step two (job-008) D5: engine-native record + replay.
//!
//! The verification artifact a screenshot reel cannot be. A browsing run is
//! recorded at the *accessibility layer* as an ordered, content-addressed ledger
//! of engine observations (the projected [`A11yTreeUpdate`] stream), the actions
//! the agent issued, and the engine's settle signals. Because the run is driven
//! deterministically, replaying the ledger through a fresh [`AccessibilityReader`]
//! reproduces the run faithfully at the accessibility layer (the PageState after
//! each observation), not as an opaque video. This is criterion 7 of job-008.
//!
//! The module is engine-agnostic: it records the same [`A11yTreeUpdate`] DTO the
//! reader consumes (not Servo types) and a `label` for each action (not the
//! `BrowserAction` enum), so a live Servo session, a fetch-cascade run, and a
//! unit test all record the same shape, and the recorder does not couple to the
//! executor's evolving action surface.
//!
//! Harness coupling (mapping a [`BrowsingRunRecord`] onto the harness run/event
//! ledger so it is replayable/forkable like an EnsembleDecision) is the named
//! integration follow-up in `theorem-harness-runtime`; the content-addressing and
//! replay/fork core live here so they are testable without the harness.

use serde::{Deserialize, Serialize};

use crate::browser_engine::PageState;
use crate::browser_perception::{A11yDiff, A11yTreeUpdate, AccessibilityReader};

/// One step in a browsing run's accessibility-layer ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BrowsingRunStep {
    /// An accessibility tree update arrived from the engine.
    Observe { update: A11yTreeUpdate },
    /// An action was issued to the engine, recorded by its display label (e.g.
    /// `"Click 42"`). A marker, so the replay shows what the agent did between
    /// observations without coupling this module to the `BrowserAction` enum.
    Action { label: String },
    /// The engine reported layout/paint quiescence (the job-008 D2 settle signal).
    Settle,
}

/// Records a browsing run at the accessibility layer: the ordered ledger of
/// engine observations, issued actions, and settle signals.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrowsingRunRecorder {
    steps: Vec<BrowsingRunStep>,
}

impl BrowsingRunRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an accessibility tree update as it arrives from the engine.
    pub fn record_observe(&mut self, update: A11yTreeUpdate) {
        self.steps.push(BrowsingRunStep::Observe { update });
    }

    /// Record that an action was issued, by its display label.
    pub fn record_action(&mut self, label: impl Into<String>) {
        self.steps.push(BrowsingRunStep::Action {
            label: label.into(),
        });
    }

    /// Record an engine settle (layout/paint quiescence) signal.
    pub fn record_settle(&mut self) {
        self.steps.push(BrowsingRunStep::Settle);
    }

    pub fn steps(&self) -> &[BrowsingRunStep] {
        &self.steps
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Seal the recording into a content-addressed [`BrowsingRunRecord`].
    pub fn finish(self) -> BrowsingRunRecord {
        BrowsingRunRecord::from_steps(self.steps)
    }
}

/// A sealed, content-addressed browsing run. `run_id` is the BLAKE3 hash of the
/// canonical-serialized step ledger, so identical runs share an id and a forked
/// run (a prefix plus new steps) gets a distinct id while sharing the prefix:
/// replayable and forkable like an EnsembleDecision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowsingRunRecord {
    pub run_id: String,
    pub steps: Vec<BrowsingRunStep>,
}

impl BrowsingRunRecord {
    /// Seal a step ledger, computing its content address.
    pub fn from_steps(steps: Vec<BrowsingRunStep>) -> Self {
        let run_id = content_address(&steps);
        Self { run_id, steps }
    }

    pub fn observe_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, BrowsingRunStep::Observe { .. }))
            .count()
    }

    pub fn action_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, BrowsingRunStep::Action { .. }))
            .count()
    }

    pub fn settle_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step, BrowsingRunStep::Settle))
            .count()
    }

    /// Replay the ledger through a fresh [`AccessibilityReader`], reproducing the
    /// run at the accessibility layer: the PageState after each Observe (and the
    /// diff each one produced), the actions in order, and the step indices where
    /// the engine signalled settle.
    pub fn replay(&self) -> BrowsingRunReplay {
        let mut reader = AccessibilityReader::new();
        let mut page_states = Vec::new();
        let mut diffs = Vec::new();
        let mut actions = Vec::new();
        let mut settle_at = Vec::new();

        for (index, step) in self.steps.iter().enumerate() {
            match step {
                BrowsingRunStep::Observe { update } => {
                    diffs.push(reader.apply_update(update.clone()));
                    page_states.push(reader.page_state());
                }
                BrowsingRunStep::Action { label } => actions.push(label.clone()),
                BrowsingRunStep::Settle => settle_at.push(index),
            }
        }

        let final_page = page_states.last().cloned();
        BrowsingRunReplay {
            run_id: self.run_id.clone(),
            page_states,
            diffs,
            actions,
            settle_at,
            final_page,
        }
    }

    /// Pair each issued action with the diff of the first observation that
    /// followed it: "what your last action changed" (job-008 D2 / criterion 3).
    /// The diff is the precise node delta (added/changed/removed), not the whole
    /// page. The opening observation (no preceding action) and actions with no
    /// following observation are omitted. When several actions precede one
    /// observation, the most recent action is credited.
    pub fn post_action_diffs(&self) -> Vec<PostActionDiff> {
        let mut reader = AccessibilityReader::new();
        let mut result = Vec::new();
        let mut pending_action: Option<String> = None;
        for step in &self.steps {
            match step {
                BrowsingRunStep::Observe { update } => {
                    let diff = reader.apply_update(update.clone());
                    if let Some(action) = pending_action.take() {
                        result.push(PostActionDiff { action, diff });
                    }
                }
                BrowsingRunStep::Action { label } => pending_action = Some(label.clone()),
                BrowsingRunStep::Settle => {}
            }
        }
        result
    }

    /// Fork this run: take its first `prefix_len` steps, then append `extra`.
    /// The fork shares the prefix and gets its own content address. `prefix_len`
    /// is clamped to the run length.
    pub fn fork(&self, prefix_len: usize, extra: Vec<BrowsingRunStep>) -> BrowsingRunRecord {
        let take = prefix_len.min(self.steps.len());
        let mut steps: Vec<BrowsingRunStep> = self.steps.iter().take(take).cloned().collect();
        steps.extend(extra);
        BrowsingRunRecord::from_steps(steps)
    }
}

/// The result of replaying a [`BrowsingRunRecord`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrowsingRunReplay {
    pub run_id: String,
    /// The PageState reproduced after each Observe step, in order.
    pub page_states: Vec<PageState>,
    /// The diff returned by each Observe's `apply_update`, in order.
    pub diffs: Vec<A11yDiff>,
    /// The action labels in order (the agent's moves between observations).
    pub actions: Vec<String>,
    /// Step indices at which the engine signalled settle.
    pub settle_at: Vec<usize>,
    /// The final reproduced PageState (the last Observe), if the run had any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_page: Option<PageState>,
}

/// One issued action paired with the precise node delta it caused (the diff of
/// the observation that followed it). The verification form of "this is what your
/// click did", not "here is the whole page again".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostActionDiff {
    pub action: String,
    pub diff: A11yDiff,
}

/// Content-address a step ledger: BLAKE3 over the deterministic JSON encoding.
/// The recorded types serialize in a fixed order (structs in declaration order,
/// `Vec`s in order, no maps), so the encoding is stable and the id reproducible.
fn content_address(steps: &[BrowsingRunStep]) -> String {
    let canonical = serde_json::to_vec(steps).unwrap_or_default();
    let hash = blake3::hash(&canonical);
    format!("browsing-run:{}", hash.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_perception::{A11yNode, A11yRect, A11yTreeUpdate};

    fn node(id: u64, role: &str) -> A11yNode {
        A11yNode {
            id,
            role: role.to_string(),
            ..A11yNode::default()
        }
    }

    /// A root + a heading carrying `text`, plus a button. Returns the full tree.
    fn page(text: &str, button_label: &str) -> A11yTreeUpdate {
        let mut heading = node(2, "Heading");
        heading.value = Some(text.to_string());

        let mut button = node(3, "Button");
        button.label = Some(button_label.to_string());
        button.bounds = Some(A11yRect {
            x0: 0.0,
            y0: 40.0,
            x1: 60.0,
            y1: 60.0,
        });

        let mut root = node(1, "RootWebArea");
        root.children = vec![2, 3];

        A11yTreeUpdate {
            nodes: vec![root, heading, button],
            root: Some(1),
            focus: Some(1),
            url: Some("https://example.com/p".to_string()),
            title: Some("P".to_string()),
        }
    }

    fn sample_record() -> BrowsingRunRecord {
        let mut recorder = BrowsingRunRecorder::new();
        recorder.record_observe(page("first", "Save"));
        recorder.record_action("Click 3");
        recorder.record_settle();
        // Only the button label changed: an incremental update with one node.
        let mut changed_button = node(3, "Button");
        changed_button.label = Some("Saved".to_string());
        changed_button.bounds = Some(A11yRect {
            x0: 0.0,
            y0: 40.0,
            x1: 60.0,
            y1: 60.0,
        });
        recorder.record_observe(A11yTreeUpdate {
            nodes: vec![changed_button],
            ..A11yTreeUpdate::default()
        });
        recorder.record_settle();
        recorder.finish()
    }

    #[test]
    fn content_address_is_deterministic_and_order_sensitive() {
        let a = sample_record();
        let b = sample_record();
        assert_eq!(a.run_id, b.run_id, "same steps -> same content id");
        assert!(a.run_id.starts_with("browsing-run:"));

        // A different ledger (one fewer step) gets a different id.
        let mut recorder = BrowsingRunRecorder::new();
        recorder.record_observe(page("first", "Save"));
        let shorter = recorder.finish();
        assert_ne!(a.run_id, shorter.run_id);
    }

    #[test]
    fn replay_reproduces_the_run_at_the_accessibility_layer() {
        let record = sample_record();
        assert_eq!(record.observe_count(), 2);
        assert_eq!(record.action_count(), 1);
        assert_eq!(record.settle_count(), 2);

        let replay = record.replay();
        // One PageState per Observe, in order.
        assert_eq!(replay.page_states.len(), 2);
        assert_eq!(replay.actions, vec!["Click 3".to_string()]);
        // Settles were at step indices 2 and 4.
        assert_eq!(replay.settle_at, vec![2, 4]);

        // The first observation surfaced the heading text + button.
        assert!(replay.page_states[0].distilled_text.contains("first"));
        let button0 = replay.page_states[0]
            .interactive_elements
            .iter()
            .find(|el| el.element_id == "3")
            .expect("button present in first observation");
        assert_eq!(button0.name, "Save");

        // The second (incremental) observation reflects the mutated button,
        // carried forward with the unchanged heading still present.
        let final_page = replay.final_page.as_ref().expect("final page");
        let button1 = final_page
            .interactive_elements
            .iter()
            .find(|el| el.element_id == "3")
            .expect("button present in final observation");
        assert_eq!(button1.name, "Saved");
        assert!(final_page.distilled_text.contains("first"));

        // The second observe's diff reports the button as changed, nothing added/removed.
        assert_eq!(replay.diffs[1].changed, vec![3]);
        assert!(replay.diffs[1].added.is_empty());
        assert!(replay.diffs[1].removed.is_empty());
    }

    #[test]
    fn replay_is_faithful_to_a_live_reader() {
        let record = sample_record();
        let replay = record.replay();

        // Applying the same observed updates directly to a fresh reader must
        // reproduce the same final PageState (deterministic at the a11y layer).
        let mut live = AccessibilityReader::new();
        for step in &record.steps {
            if let BrowsingRunStep::Observe { update } = step {
                live.apply_update(update.clone());
            }
        }
        assert_eq!(replay.final_page, Some(live.page_state()));
    }

    #[test]
    fn fork_shares_prefix_and_gets_a_new_id() {
        let base = sample_record();
        // Fork after the first observe + action (2 steps), then take a different
        // branch: a settle and a fresh observation.
        let mut branch_button = node(3, "Button");
        branch_button.label = Some("Cancelled".to_string());
        branch_button.bounds = Some(A11yRect {
            x0: 0.0,
            y0: 40.0,
            x1: 60.0,
            y1: 60.0,
        });
        let forked = base.fork(
            2,
            vec![
                BrowsingRunStep::Settle,
                BrowsingRunStep::Observe {
                    update: A11yTreeUpdate {
                        nodes: vec![branch_button],
                        ..A11yTreeUpdate::default()
                    },
                },
            ],
        );

        assert_ne!(forked.run_id, base.run_id);
        // Prefix shared: first two steps identical.
        assert_eq!(&forked.steps[0..2], &base.steps[0..2]);
        // The fork replays its own branch.
        let replay = forked.replay();
        assert_eq!(
            replay
                .final_page
                .unwrap()
                .interactive_elements
                .iter()
                .find(|el| el.element_id == "3")
                .unwrap()
                .name,
            "Cancelled"
        );
    }

    #[test]
    fn post_action_diffs_pair_each_action_with_its_precise_delta() {
        let record = sample_record();
        let pairs = record.post_action_diffs();
        // Only the post-action observation is paired; the opening load is not.
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].action, "Click 3");
        // Precise delta: only node 3 changed, nothing added/removed, no full page.
        assert_eq!(pairs[0].diff.changed, vec![3]);
        assert!(pairs[0].diff.added.is_empty());
        assert!(pairs[0].diff.removed.is_empty());
    }

    #[test]
    fn empty_run_replays_to_nothing() {
        let record = BrowsingRunRecorder::new().finish();
        let replay = record.replay();
        assert!(replay.page_states.is_empty());
        assert!(replay.final_page.is_none());
        assert!(replay.actions.is_empty());
    }

    #[test]
    fn record_round_trips_through_serde() {
        let record = sample_record();
        let json = serde_json::to_string(&record).expect("serialize");
        let restored: BrowsingRunRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, restored);
        // The restored record content-addresses to the same id.
        assert_eq!(
            restored.run_id,
            BrowsingRunRecord::from_steps(restored.steps.clone()).run_id
        );
    }
}
