//! The per-binding budget governor (spec Part 3 budget plane, build-order
//! step 6).
//!
//! `BUDGET.ALLOCATED` caps how much a run may allocate; this module is the
//! other half: a hard runtime guard that meters every head contribution so
//! "no head wakes unless the binding allows" is enforced, not hoped for. The
//! governor is a pure check (`check_contribution_budget`) plus a pure mutation
//! (`apply_contribution_charge`) over a serializable `BindingBudgetState` that
//! travels on the `AgentBinding`. The kernel guard calls the check before a
//! `HEADS.CONTRIBUTE` transition and applies the charge as part of it.
//!
//! Three caps are enforced, all derived from the binding's `BindingBudgetScope`:
//! the run total (allocated units, falling back to the shared budget), the
//! optional per-head limit, and `max_parallel_heads` (read here as a bound on
//! the number of distinct heads that may wake in a run, the append-only
//! lifecycle's faithful proxy for concurrent wakes).

use crate::agent_binding::{BindingBudgetScope, BindingError};
use crate::types::{GuardViolation, Payload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The live spend ledger for a binding run. Persisted as part of the binding so
/// it survives a store reopen; `BTreeMap` keeps the per-head ledger
/// deterministic for state hashing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingBudgetState {
    #[serde(default)]
    pub spent_total: f64,
    #[serde(default)]
    pub spent_per_head: BTreeMap<String, f64>,
}

impl BindingBudgetState {
    /// Distinct heads that have spent any units this run. Used as the
    /// `max_parallel_heads` measure.
    pub fn active_head_count(&self) -> usize {
        self.spent_per_head.len()
    }
}

/// Verify that charging `units` to `head_id` stays inside every cap. Read-only;
/// the kernel calls this from the `HEADS.CONTRIBUTE` guard before the
/// transition is applied. Returns a `GuardViolation` (the same enforced-guard
/// channel as the rest of the binding machine) on breach.
pub fn check_contribution_budget(
    scope: &BindingBudgetScope,
    state: &BindingBudgetState,
    head_id: &str,
    units: f64,
) -> Result<(), BindingError> {
    if units < 0.0 {
        return Err(guard(
            "invalid_contribution_cost",
            "contribution cost_units cannot be negative",
            json!({ "head_id": head_id, "units": units }),
        ));
    }

    let run_cap = if scope.allocated_run_budget_units > 0.0 {
        scope.allocated_run_budget_units
    } else {
        scope.shared_budget_units
    };
    if state.spent_total + units > run_cap {
        return Err(guard(
            "binding_budget_overspent",
            "HEADS.CONTRIBUTE would exceed the run budget",
            json!({
                "head_id": head_id,
                "units": units,
                "spent_total": state.spent_total,
                "run_cap": run_cap,
            }),
        ));
    }

    if let Some(limit) = scope
        .per_head_limits
        .iter()
        .find(|limit| limit.head_id == head_id)
    {
        let head_spent = state.spent_per_head.get(head_id).copied().unwrap_or(0.0);
        if head_spent + units > limit.max_units {
            return Err(guard(
                "head_budget_overspent",
                "HEADS.CONTRIBUTE would exceed the per-head budget limit",
                json!({
                    "head_id": head_id,
                    "units": units,
                    "head_spent": head_spent,
                    "head_cap": limit.max_units,
                }),
            ));
        }
    }

    let is_new_head = !state.spent_per_head.contains_key(head_id);
    if is_new_head && state.active_head_count() + 1 > scope.max_parallel_heads {
        return Err(guard(
            "max_parallel_heads_exceeded",
            "waking this head would exceed max_parallel_heads",
            json!({
                "head_id": head_id,
                "active_heads": state.active_head_count(),
                "max_parallel_heads": scope.max_parallel_heads,
            }),
        ));
    }

    Ok(())
}

/// Record a charge after `check_contribution_budget` has passed. Pure mutation;
/// the kernel calls this from the `HEADS.CONTRIBUTE` payload application.
pub fn apply_contribution_charge(state: &mut BindingBudgetState, head_id: &str, units: f64) {
    if units <= 0.0 {
        // A zero-cost contribution still records the head as active so
        // max_parallel_heads counts it.
        state
            .spent_per_head
            .entry(head_id.to_string())
            .or_insert(0.0);
        return;
    }
    state.spent_total += units;
    *state
        .spent_per_head
        .entry(head_id.to_string())
        .or_insert(0.0) += units;
}

fn guard(code: &str, message: &str, details: Value) -> BindingError {
    let details = match details {
        Value::Object(map) => map,
        _ => Payload::new(),
    };
    BindingError::Guard(Box::new(GuardViolation {
        code: code.to_string(),
        message: message.to_string(),
        policy_layer: crate::constitution::GLOBAL_LAW_LAYER.to_string(),
        required_state: String::new(),
        received_state: String::new(),
        missing_fields: Vec::new(),
        details,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_binding::HeadBudgetLimit;

    fn scope() -> BindingBudgetScope {
        let mut scope = BindingBudgetScope::new("theorem", 100.0, 2);
        scope.allocated_run_budget_units = 30.0;
        scope.per_head_limits = vec![HeadBudgetLimit {
            head_id: "claude".to_string(),
            max_units: 10.0,
        }];
        scope
    }

    fn err_code(error: BindingError) -> String {
        match error {
            BindingError::Guard(violation) => violation.code,
        }
    }

    #[test]
    fn within_budget_passes_and_charges() {
        let scope = scope();
        let mut state = BindingBudgetState::default();
        check_contribution_budget(&scope, &state, "claude", 5.0).unwrap();
        apply_contribution_charge(&mut state, "claude", 5.0);
        assert_eq!(state.spent_total, 5.0);
        assert_eq!(state.spent_per_head.get("claude"), Some(&5.0));
        check_contribution_budget(&scope, &state, "claude", 4.0).unwrap();
    }

    #[test]
    fn run_total_cap_is_enforced() {
        let scope = scope();
        let mut state = BindingBudgetState::default();
        apply_contribution_charge(&mut state, "deepseek", 28.0);
        let error = check_contribution_budget(&scope, &state, "deepseek", 5.0).unwrap_err();
        assert_eq!(err_code(error), "binding_budget_overspent");
    }

    #[test]
    fn per_head_cap_is_enforced() {
        let scope = scope();
        let mut state = BindingBudgetState::default();
        apply_contribution_charge(&mut state, "claude", 8.0);
        let error = check_contribution_budget(&scope, &state, "claude", 5.0).unwrap_err();
        assert_eq!(err_code(error), "head_budget_overspent");
    }

    #[test]
    fn max_parallel_heads_is_enforced() {
        let scope = scope(); // max_parallel_heads = 2
        let mut state = BindingBudgetState::default();
        apply_contribution_charge(&mut state, "claude", 1.0);
        apply_contribution_charge(&mut state, "deepseek", 1.0);
        // A third distinct head exceeds the parallel bound.
        let error = check_contribution_budget(&scope, &state, "qwen", 1.0).unwrap_err();
        assert_eq!(err_code(error), "max_parallel_heads_exceeded");
        // An already-active head is fine.
        check_contribution_budget(&scope, &state, "claude", 1.0).unwrap();
    }

    #[test]
    fn negative_cost_is_rejected() {
        let scope = scope();
        let state = BindingBudgetState::default();
        let error = check_contribution_budget(&scope, &state, "claude", -1.0).unwrap_err();
        assert_eq!(err_code(error), "invalid_contribution_cost");
    }
}
