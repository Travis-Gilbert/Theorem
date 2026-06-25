use std::collections::BTreeSet;
use std::sync::Arc;

use rustyred_thg_core::{
    HookContext, HookError, HookHandler, HookOutcome, HookRegistration, MutationEvent,
    MutationKind, MutationMatcher,
};

use super::ambient::refresh_code_compiler_artifacts_for_repo;
use crate::{
    property_string, CALLS_SYMBOL, CENTRALITY_PROPERTY, CODE_SYMBOL_LABEL, DECLARES_SYMBOL,
    DEPENDS_ON_SYMBOL, EMBEDDING_PROPERTY,
};

/// Keep compiler artifacts warm as code graph structure changes.
///
/// This is intentionally an ambient perception hook. If a repo has no compiled
/// spec yet, the hook bootstraps one from the current graph. If a spec already
/// exists, it refreshes drift findings against that spec instead of overwriting
/// the oracle.
pub fn incremental_code_compiler_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(code_compiler_handler);
    HookRegistration::new(
        "code.incremental_compiler",
        MutationMatcher::any()
            .with_kinds([
                MutationKind::NodeUpserted,
                MutationKind::EdgeUpserted,
                MutationKind::EdgeDeleted,
            ])
            .with_labels([
                CODE_SYMBOL_LABEL,
                CALLS_SYMBOL,
                DEPENDS_ON_SYMBOL,
                DECLARES_SYMBOL,
            ]),
        coalesce_code_compiler,
        handler,
    )
}

fn coalesce_code_compiler(_event: &MutationEvent) -> Option<String> {
    Some("code-compiler".to_string())
}

fn code_compiler_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    let mut repos = BTreeSet::new();
    for event in events {
        match event.kind {
            MutationKind::NodeUpserted => {
                if is_derived_symbol_update(&event.changed_props) {
                    continue;
                }
                add_symbol_repo(ctx, &event.id, &mut repos)?;
            }
            MutationKind::EdgeUpserted | MutationKind::EdgeDeleted => {
                if let Some(edge) = ctx.store.get_edge(&event.id).map_err(HookError::from)? {
                    add_symbol_repo(ctx, &edge.from_id, &mut repos)?;
                    add_symbol_repo(ctx, &edge.to_id, &mut repos)?;
                }
            }
            MutationKind::NodeDeleted => {}
        }
    }
    if repos.is_empty() {
        return Ok(HookOutcome::Done);
    }

    let mut writes = 0usize;
    for (tenant_id, repo_id) in repos {
        let readout = refresh_code_compiler_artifacts_for_repo(ctx.store, &tenant_id, &repo_id)
            .map_err(HookError::from)?;
        writes += 1 + readout.drift_count();
    }
    Ok(HookOutcome::Wrote { mutations: writes })
}

fn add_symbol_repo(
    ctx: &HookContext,
    symbol_id: &str,
    repos: &mut BTreeSet<(String, String)>,
) -> Result<(), HookError> {
    let Some(node) = ctx.store.get_node(symbol_id).map_err(HookError::from)? else {
        return Ok(());
    };
    if !node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL) {
        return Ok(());
    }
    let Some(repo_id) = property_string(&node.properties, "repo_id") else {
        return Ok(());
    };
    let tenant_id = property_string(&node.properties, "tenant_id").unwrap_or_default();
    if tenant_id.trim().is_empty() || repo_id.trim().is_empty() {
        return Ok(());
    }
    repos.insert((tenant_id, repo_id));
    Ok(())
}

fn is_derived_symbol_update(changed_props: &[String]) -> bool {
    !changed_props.is_empty()
        && changed_props.iter().all(|prop| {
            prop == CENTRALITY_PROPERTY
                || prop == EMBEDDING_PROPERTY
                || prop == "epistemic_readout"
                || prop == "epistemic_score"
        })
}
