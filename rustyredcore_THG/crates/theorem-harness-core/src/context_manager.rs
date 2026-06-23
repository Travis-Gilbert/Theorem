use serde::{Deserialize, Serialize};

pub const COMPACTION_TRIGGER_THRESHOLD: f64 = 0.90;
pub const HARD_LIMIT_THRESHOLD: f64 = 0.95;
pub const MAX_CONSECUTIVE_REDUCTION_FAILURES: u8 = 3;
pub const CLEARED_TOOL_RESULT_PLACEHOLDER: &str = "[Old tool result content cleared]";
pub const DEFAULT_KEEP_RECENT_TOOL_RESULTS: usize = 5;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub context_window: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub tool_result_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ContextCheckResult {
    Ok,
    ReductionNeeded,
    ContextExhausted { utilization_pct: u8, reason: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextGuard {
    pub last_input_tokens: u64,
    pub last_output_tokens: u64,
    pub context_window: u64,
    pub consecutive_reduction_failures: u8,
    pub reduction_disabled: bool,
}

impl ContextGuard {
    pub fn with_context_window(context_window: u64) -> Self {
        Self {
            context_window,
            ..Self::default()
        }
    }

    pub fn record_usage(&mut self, usage: &ProviderUsage) {
        self.last_input_tokens = usage.input_tokens;
        self.last_output_tokens = usage.output_tokens;
        if usage.context_window > 0 {
            self.context_window = usage.context_window;
        }
    }

    pub fn utilization(&self) -> Option<f64> {
        if self.context_window == 0 {
            return None;
        }
        Some((self.last_input_tokens + self.last_output_tokens) as f64 / self.context_window as f64)
    }

    pub fn check(&self) -> ContextCheckResult {
        let Some(utilization) = self.utilization() else {
            return ContextCheckResult::Ok;
        };
        if utilization >= HARD_LIMIT_THRESHOLD && self.reduction_disabled {
            return ContextCheckResult::ContextExhausted {
                utilization_pct: pct(utilization),
                reason: format!(
                    "Context {}% full; reduction disabled after {} consecutive failures",
                    pct(utilization),
                    self.consecutive_reduction_failures
                ),
            };
        }
        if utilization >= COMPACTION_TRIGGER_THRESHOLD && !self.reduction_disabled {
            return ContextCheckResult::ReductionNeeded;
        }
        ContextCheckResult::Ok
    }

    pub fn record_reduction_success(&mut self) {
        self.consecutive_reduction_failures = 0;
        self.reduction_disabled = false;
    }

    pub fn record_reduction_failure(&mut self) {
        self.consecutive_reduction_failures = self.consecutive_reduction_failures.saturating_add(1);
        if self.consecutive_reduction_failures >= MAX_CONSECUTIVE_REDUCTION_FAILURES {
            self.reduction_disabled = true;
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
    },
    AssistantToolCalls {
        #[serde(default)]
        content: Option<String>,
        tool_calls: Vec<ToolCallEnvelope>,
    },
    ToolResults {
        results: Vec<ToolResultEnvelope>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCallEnvelope {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEnvelope {
    pub tool_call_id: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MicrocompactStats {
    pub envelopes_cleared: usize,
    pub entries_cleared: usize,
    pub bytes_freed: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ContextReduction {
    NoOp,
    MessageTrimmed {
        removed_messages: usize,
    },
    Microcompacted(MicrocompactStats),
    Autocompacted {
        summary_tokens: i64,
        kept_tail_messages: usize,
    },
    AutocompactionNeeded {
        utilization_pct: u8,
    },
    ContextExhausted {
        utilization_pct: u8,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextManagerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_keep_recent_tool_results")]
    pub keep_recent_tool_results: usize,
    #[serde(default)]
    pub max_messages: usize,
    #[serde(default = "default_keep_recent_messages")]
    pub keep_recent_messages: usize,
}

impl Default for ContextManagerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_recent_tool_results: DEFAULT_KEEP_RECENT_TOOL_RESULTS,
            max_messages: 0,
            keep_recent_messages: default_keep_recent_messages(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextManagerStats {
    pub utilization_pct: Option<u8>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_window: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub tool_result_bytes: u64,
    pub reduction_disabled: bool,
    pub consecutive_reduction_failures: u8,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextManager {
    pub config: ContextManagerConfig,
    pub guard: ContextGuard,
    pub last_usage: ProviderUsage,
}

impl ContextManager {
    pub fn new(config: ContextManagerConfig) -> Self {
        Self {
            config,
            guard: ContextGuard::default(),
            last_usage: ProviderUsage::default(),
        }
    }

    pub fn record_usage(&mut self, usage: ProviderUsage) {
        self.guard.record_usage(&usage);
        self.last_usage = usage;
    }

    pub fn stats(&self) -> ContextManagerStats {
        ContextManagerStats {
            utilization_pct: self.guard.utilization().map(pct),
            input_tokens: self.last_usage.input_tokens,
            output_tokens: self.last_usage.output_tokens,
            context_window: self.guard.context_window,
            cache_read_tokens: self.last_usage.cache_read_tokens,
            cache_write_tokens: self.last_usage.cache_write_tokens,
            tool_result_bytes: self.last_usage.tool_result_bytes,
            reduction_disabled: self.guard.reduction_disabled,
            consecutive_reduction_failures: self.guard.consecutive_reduction_failures,
        }
    }

    pub fn reduce_before_call(
        &mut self,
        history: &mut Vec<ContextMessage>,
        summary: Option<String>,
    ) -> ContextReduction {
        if !self.config.enabled {
            return ContextReduction::NoOp;
        }
        if self.config.max_messages > 0 && history.len() > self.config.max_messages {
            let removed = trim_message_count(history, self.config.max_messages);
            if removed > 0 {
                self.guard.record_reduction_success();
                return ContextReduction::MessageTrimmed {
                    removed_messages: removed,
                };
            }
        }

        match self.guard.check() {
            ContextCheckResult::Ok => ContextReduction::NoOp,
            ContextCheckResult::ContextExhausted {
                utilization_pct,
                reason,
            } => ContextReduction::ContextExhausted {
                utilization_pct,
                reason,
            },
            ContextCheckResult::ReductionNeeded => {
                let stats = microcompact(history, self.config.keep_recent_tool_results);
                if stats.envelopes_cleared > 0 {
                    self.guard.record_reduction_success();
                    return ContextReduction::Microcompacted(stats);
                }

                if let Some(summary) = summary.filter(|value| !value.trim().is_empty()) {
                    let reduction = autocompact_with_summary(
                        history,
                        summary,
                        self.config.keep_recent_messages,
                    );
                    self.guard.record_reduction_success();
                    return reduction;
                }

                self.guard.record_reduction_failure();
                ContextReduction::AutocompactionNeeded {
                    utilization_pct: self.guard.utilization().map(pct).unwrap_or(0),
                }
            }
        }
    }
}

pub fn microcompact(history: &mut [ContextMessage], keep_recent: usize) -> MicrocompactStats {
    let mut tool_result_indices = history
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            matches!(message, ContextMessage::ToolResults { .. }).then_some(index)
        })
        .collect::<Vec<_>>();
    if tool_result_indices.len() <= keep_recent {
        return MicrocompactStats::default();
    }
    let cut = tool_result_indices.len().saturating_sub(keep_recent);
    tool_result_indices.truncate(cut);

    let mut stats = MicrocompactStats::default();
    for index in tool_result_indices {
        let ContextMessage::ToolResults { results } = &mut history[index] else {
            continue;
        };
        let mut envelope_changed = false;
        for result in results {
            if result.content == CLEARED_TOOL_RESULT_PLACEHOLDER {
                continue;
            }
            let old_len = result.content.len();
            result.content = CLEARED_TOOL_RESULT_PLACEHOLDER.to_string();
            stats.bytes_freed += old_len.saturating_sub(CLEARED_TOOL_RESULT_PLACEHOLDER.len());
            stats.entries_cleared += 1;
            envelope_changed = true;
        }
        if envelope_changed {
            stats.envelopes_cleared += 1;
        }
    }
    stats
}

pub fn autocompact_with_summary(
    history: &mut Vec<ContextMessage>,
    summary: String,
    keep_recent_messages: usize,
) -> ContextReduction {
    if summary.trim().is_empty() || history.len() <= keep_recent_messages {
        return ContextReduction::NoOp;
    }
    let split = snap_split_forward(history, history.len().saturating_sub(keep_recent_messages));
    if split == 0 || split >= history.len() {
        return ContextReduction::NoOp;
    }
    let tail = history.split_off(split);
    let summary_tokens = calibrated_summary_tokens(&summary);
    history.clear();
    history.push(ContextMessage::Assistant {
        content: format!("Context summary:\n\n{}", summary.trim()),
    });
    history.extend(tail);
    ContextReduction::Autocompacted {
        summary_tokens,
        kept_tail_messages: history.len().saturating_sub(1),
    }
}

fn trim_message_count(history: &mut Vec<ContextMessage>, max_messages: usize) -> usize {
    let mut removed = 0;
    while history.len() > max_messages {
        let remove_count = snap_remove_count(history);
        history.drain(0..remove_count);
        removed += remove_count;
    }
    removed
}

fn snap_remove_count(history: &[ContextMessage]) -> usize {
    if history.len() >= 2
        && matches!(history[0], ContextMessage::AssistantToolCalls { .. })
        && matches!(history[1], ContextMessage::ToolResults { .. })
    {
        2
    } else {
        1
    }
}

fn snap_split_forward(history: &[ContextMessage], mut split: usize) -> usize {
    if split > 0
        && split < history.len()
        && matches!(
            history[split - 1],
            ContextMessage::AssistantToolCalls { .. }
        )
        && matches!(history[split], ContextMessage::ToolResults { .. })
    {
        return split - 1;
    }
    while split < history.len() {
        if split > 0
            && matches!(
                history[split - 1],
                ContextMessage::AssistantToolCalls { .. }
            )
            && matches!(history[split], ContextMessage::ToolResults { .. })
        {
            split += 1;
            continue;
        }
        return split;
    }
    history.len()
}

fn calibrated_summary_tokens(summary: &str) -> i64 {
    let bytes = summary.trim().len() as i64;
    if bytes == 0 {
        0
    } else {
        ((bytes + 3) / 4).max(1)
    }
}

fn pct(value: f64) -> u8 {
    (value * 100.0).round().clamp(0.0, 100.0) as u8
}

fn default_true() -> bool {
    true
}

fn default_keep_recent_tool_results() -> usize {
    DEFAULT_KEEP_RECENT_TOOL_RESULTS
}

fn default_keep_recent_messages() -> usize {
    12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str) -> ContextMessage {
        ContextMessage::AssistantToolCalls {
            content: None,
            tool_calls: vec![ToolCallEnvelope {
                id: id.to_string(),
                name: "tool".to_string(),
                arguments: "{}".to_string(),
            }],
        }
    }

    fn result(id: &str, content: &str) -> ContextMessage {
        ContextMessage::ToolResults {
            results: vec![ToolResultEnvelope {
                tool_call_id: id.to_string(),
                content: content.to_string(),
            }],
        }
    }

    #[test]
    fn guard_is_noop_when_context_window_is_unknown() {
        let guard = ContextGuard::default();
        assert_eq!(guard.check(), ContextCheckResult::Ok);
    }

    #[test]
    fn guard_trips_circuit_after_three_failures() {
        let mut guard = ContextGuard::with_context_window(100);
        guard.record_usage(&ProviderUsage {
            input_tokens: 96,
            context_window: 100,
            ..ProviderUsage::default()
        });
        guard.record_reduction_failure();
        guard.record_reduction_failure();
        assert!(!guard.reduction_disabled);
        guard.record_reduction_failure();
        assert!(matches!(
            guard.check(),
            ContextCheckResult::ContextExhausted { .. }
        ));
    }

    #[test]
    fn microcompact_preserves_tool_result_envelopes_and_is_idempotent() {
        let body = "x".repeat(2_000);
        let mut history = vec![
            call("a"),
            result("a", &body),
            call("b"),
            result("b", "recent"),
        ];
        let first = microcompact(&mut history, 1);
        assert_eq!(first.envelopes_cleared, 1);
        assert_eq!(first.entries_cleared, 1);
        assert!(matches!(history[1], ContextMessage::ToolResults { .. }));
        let second = microcompact(&mut history, 1);
        assert_eq!(second, MicrocompactStats::default());
    }

    #[test]
    fn manager_requests_autocompaction_when_microcompact_cannot_free_bytes() {
        let mut manager = ContextManager::new(ContextManagerConfig::default());
        manager.record_usage(ProviderUsage {
            input_tokens: 91,
            context_window: 100,
            ..ProviderUsage::default()
        });
        let mut history = vec![ContextMessage::User {
            content: "hello".to_string(),
        }];
        assert_eq!(
            manager.reduce_before_call(&mut history, None),
            ContextReduction::AutocompactionNeeded {
                utilization_pct: 91
            }
        );
    }

    #[test]
    fn autocompact_rewrites_head_and_keeps_tail_without_splitting_pair() {
        let mut history = vec![
            ContextMessage::User {
                content: "old".to_string(),
            },
            call("a"),
            result("a", "old result"),
            ContextMessage::User {
                content: "recent".to_string(),
            },
        ];
        let reduction = autocompact_with_summary(&mut history, "summary".to_string(), 2);
        assert!(matches!(
            reduction,
            ContextReduction::Autocompacted {
                kept_tail_messages: 3,
                ..
            }
        ));
        assert!(matches!(history[0], ContextMessage::Assistant { .. }));
        assert!(matches!(
            history[1],
            ContextMessage::AssistantToolCalls { .. }
        ));
        assert!(matches!(history[2], ContextMessage::ToolResults { .. }));
    }
}
