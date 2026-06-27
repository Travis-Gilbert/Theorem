use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::stable_hash;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    compile_engineering_in_memory, EngineeringCompileInput, EngineeringCompileOutput,
    EvidenceAuthority, EvidenceSource, ObservedArchitectureInput, ObservedBehaviorInput,
    ObservedImplementationObligationInput, ObservedValidatorSpecInput, ProgramAnalysisOutput,
    TaintMark, TraceEventFact, TraceEventKind, TraceSnapshotFact,
};

const TRACE_ENGINEERING_COMPILER_VERSION: &str = "theorem-trace-to-contract-v0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TraceToEngineeringOptions {
    pub repo_id: String,
    pub include_read_obligations: bool,
}

impl TraceToEngineeringOptions {
    pub fn new(repo_id: impl Into<String>) -> Self {
        Self {
            repo_id: repo_id.into(),
            include_read_obligations: false,
        }
    }
}

pub fn program_analysis_trace_to_engineering_input(
    analysis: &ProgramAnalysisOutput,
    options: TraceToEngineeringOptions,
) -> EngineeringCompileInput {
    let mut input = EngineeringCompileInput::new(&analysis.run.tenant_id, options.repo_id);
    input.compiler_version = TRACE_ENGINEERING_COMPILER_VERSION.to_string();

    if !analysis.runtime_traces.is_empty() {
        input.architecture_inputs.push(ObservedArchitectureInput {
            component_name: "runtime-trace-replay".to_string(),
            description: "Replay dynamic trace snapshots, schedules, and observed events as reconstruction evidence.".to_string(),
            evidence_ids: analysis
                .runtime_traces
                .iter()
                .flat_map(|trace| evidence_ids(&trace.trace_id, &trace.evidence_ids))
                .collect(),
            validator_refs: vec!["trace-schedule-replay".to_string()],
            unknowns: Vec::new(),
        });
    }
    if !analysis.taint_marks.is_empty() {
        input.architecture_inputs.push(ObservedArchitectureInput {
            component_name: "taint-provenance-replay".to_string(),
            description: "Preserve input influence and indirect read/write provenance recovered from the p-code taint domain.".to_string(),
            evidence_ids: analysis
                .taint_marks
                .iter()
                .flat_map(|mark| evidence_ids(&mark.taint_id, &mark.evidence_ids))
                .collect(),
            validator_refs: vec!["taint-presence".to_string()],
            unknowns: vec![
                "Taint propagation precision depends on the trace oracle and p-code coverage."
                    .to_string(),
            ],
        });
    }

    let snapshots_by_trace = snapshots_by_trace(&analysis.trace_snapshots);
    let events_by_trace = events_by_trace(&analysis.trace_events);
    let taints_by_trace = taints_by_trace(&analysis.taint_marks);
    for trace in &analysis.runtime_traces {
        let snapshots = snapshots_by_trace
            .get(&trace.trace_id)
            .map_or([].as_slice(), Vec::as_slice);
        let events = events_by_trace
            .get(&trace.trace_id)
            .map_or([].as_slice(), Vec::as_slice);
        let taints = taints_by_trace
            .get(&trace.trace_id)
            .map_or([].as_slice(), Vec::as_slice);
        let evidence = evidence_ids(&trace.trace_id, &trace.evidence_ids);
        input.behavior_inputs.push(ObservedBehaviorInput {
            behavior_name: format!("Replay {}", trace.trace_id),
            description: format!(
                "Runtime trace captured by {} for language {} and compiler {} has {} snapshots, {} events, and {} taint marks.",
                trace.capture_source,
                trace.language_id.as_deref().unwrap_or("unknown"),
                trace.compiler_spec_id.as_deref().unwrap_or("unknown"),
                snapshots.len(),
                events.len(),
                taints.len(),
            ),
            evidence_ids: evidence.clone(),
            validator_refs: vec![trace_validator_ref(&trace.trace_id, "trace-schedule-replay")],
            unknowns: Vec::new(),
        });
        input.evidence_sources.push(EvidenceSource {
            evidence_id: trace.trace_id.clone(),
            authority: observed_authority("runtime-trace"),
            summary: format!(
                "Trace {} captured by {} for artifact {}.",
                trace.trace_id, trace.capture_source, analysis.artifact.artifact_id
            ),
            confidence_score: 96,
            targets: vec![trace.trace_id.clone()],
        });
        for evidence_id in &trace.evidence_ids {
            input.evidence_sources.push(EvidenceSource {
                evidence_id: evidence_id.clone(),
                authority: observed_authority("runtime-trace-evidence"),
                summary: format!("Runtime trace evidence for {}.", trace.trace_id),
                confidence_score: 94,
                targets: vec![trace.trace_id.clone()],
            });
        }
    }

    for snapshot in &analysis.trace_snapshots {
        let evidence = evidence_ids(&snapshot.snapshot_id, &snapshot.evidence_ids);
        input.validator_inputs.push(ObservedValidatorSpecInput {
            target_id: snapshot.snapshot_id.clone(),
            validator_kind: "trace-schedule-replay".to_string(),
            rule: format!(
                "Replay schedule {} from snap {} with {} instruction steps and {} p-code steps; forked={}, stale={}.",
                snapshot.schedule.schedule,
                snapshot.schedule.snap,
                snapshot.schedule.instruction_steps,
                snapshot.schedule.pcode_steps,
                snapshot.forked,
                snapshot.stale,
            ),
            evidence_ids: evidence.clone(),
            unknowns: snapshot_unknowns(snapshot),
        });
        input.evidence_sources.push(EvidenceSource {
            evidence_id: snapshot.snapshot_id.clone(),
            authority: observed_authority("trace-snapshot"),
            summary: format!(
                "Trace snapshot {} uses schedule {}.",
                snapshot.snapshot_id, snapshot.schedule.schedule
            ),
            confidence_score: 95,
            targets: vec![snapshot.snapshot_id.clone()],
        });
    }

    for event in &analysis.trace_events {
        let evidence = evidence_ids(&event.event_id, &event.evidence_ids);
        input.behavior_inputs.push(ObservedBehaviorInput {
            behavior_name: format!("{} {}", event_kind_label(&event.kind), event.event_id),
            description: event_description(event),
            evidence_ids: evidence.clone(),
            validator_refs: vec![trace_validator_ref(&event.event_id, "trace-event-replay")],
            unknowns: event_unknowns(event),
        });
        input.validator_inputs.push(ObservedValidatorSpecInput {
            target_id: event.event_id.clone(),
            validator_kind: "trace-event-replay".to_string(),
            rule: event_replay_rule(event),
            evidence_ids: evidence.clone(),
            unknowns: event_unknowns(event),
        });
        if event_requires_obligation(&event.kind, options.include_read_obligations) {
            input
                .implementation_obligation_inputs
                .push(ObservedImplementationObligationInput {
                    target: event_target(event),
                    obligation: format!(
                        "Preserve observed {} behavior from trace event {}.",
                        event_kind_label(&event.kind),
                        event.event_id
                    ),
                    evidence_ids: evidence.clone(),
                    validator_refs: vec![trace_validator_ref(
                        &event.event_id,
                        "trace-event-replay",
                    )],
                    unknowns: event_unknowns(event),
                });
        }
        input.evidence_sources.push(EvidenceSource {
            evidence_id: event.event_id.clone(),
            authority: observed_authority("trace-event"),
            summary: event_description(event),
            confidence_score: 94,
            targets: vec![event.event_id.clone()],
        });
    }

    for mark in &analysis.taint_marks {
        let evidence = evidence_ids(&mark.taint_id, &mark.evidence_ids);
        let target = taint_target(mark);
        input
            .implementation_obligation_inputs
            .push(ObservedImplementationObligationInput {
                target: target.clone(),
                obligation: taint_obligation(mark),
                evidence_ids: evidence.clone(),
                validator_refs: vec![trace_validator_ref(&mark.taint_id, "taint-presence")],
                unknowns: taint_unknowns(mark),
            });
        input.validator_inputs.push(ObservedValidatorSpecInput {
            target_id: mark.taint_id.clone(),
            validator_kind: "taint-presence".to_string(),
            rule: taint_validator_rule(mark),
            evidence_ids: evidence.clone(),
            unknowns: taint_unknowns(mark),
        });
        input.evidence_sources.push(EvidenceSource {
            evidence_id: mark.taint_id.clone(),
            authority: derived_authority("taint-domain"),
            summary: format!(
                "Taint labels [{}] cover {} bytes at {}.",
                mark.labels.join(", "),
                mark.size,
                target
            ),
            confidence_score: 88,
            targets: vec![mark.taint_id.clone(), target],
        });
    }

    input
}

pub fn compile_program_analysis_trace_engineering_in_memory(
    analysis: &ProgramAnalysisOutput,
    options: TraceToEngineeringOptions,
) -> EngineeringCompileOutput {
    compile_engineering_in_memory(program_analysis_trace_to_engineering_input(
        analysis, options,
    ))
}

fn snapshots_by_trace(
    snapshots: &[TraceSnapshotFact],
) -> BTreeMap<String, Vec<&TraceSnapshotFact>> {
    let mut by_trace = BTreeMap::<String, Vec<&TraceSnapshotFact>>::new();
    for snapshot in snapshots {
        by_trace
            .entry(snapshot.trace_id.clone())
            .or_default()
            .push(snapshot);
    }
    by_trace
}

fn events_by_trace(events: &[TraceEventFact]) -> BTreeMap<String, Vec<&TraceEventFact>> {
    let mut by_trace = BTreeMap::<String, Vec<&TraceEventFact>>::new();
    for event in events {
        by_trace
            .entry(event.trace_id.clone())
            .or_default()
            .push(event);
    }
    by_trace
}

fn taints_by_trace(taints: &[TaintMark]) -> BTreeMap<String, Vec<&TaintMark>> {
    let mut by_trace = BTreeMap::<String, Vec<&TaintMark>>::new();
    for mark in taints {
        by_trace
            .entry(mark.trace_id.clone())
            .or_default()
            .push(mark);
    }
    by_trace
}

fn evidence_ids(primary_id: &str, evidence_ids: &[String]) -> Vec<String> {
    let mut ids = BTreeSet::from([primary_id.to_string()]);
    ids.extend(evidence_ids.iter().cloned());
    ids.into_iter().collect()
}

fn observed_authority(role: &str) -> EvidenceAuthority {
    EvidenceAuthority {
        authority_id: format!("program-analysis:{role}"),
        org: "Theorem".to_string(),
        role: "observed_fact".to_string(),
    }
}

fn derived_authority(role: &str) -> EvidenceAuthority {
    EvidenceAuthority {
        authority_id: format!("program-analysis:{role}"),
        org: "Theorem".to_string(),
        role: "derived_fact".to_string(),
    }
}

fn trace_validator_ref(target_id: &str, validator_kind: &str) -> String {
    format!(
        "trace:validator:{}",
        stable_hash(json!([target_id, validator_kind]))
    )
}

fn snapshot_unknowns(snapshot: &TraceSnapshotFact) -> Vec<String> {
    let mut unknowns = Vec::new();
    if snapshot.stale {
        unknowns.push("Snapshot was marked stale by the trace oracle.".to_string());
    }
    if snapshot.schedule.pcode_steps > 0 {
        unknowns.push(
            "P-code step replay must preserve partial-instruction schedule semantics.".to_string(),
        );
    }
    unknowns
}

fn event_unknowns(event: &TraceEventFact) -> Vec<String> {
    let mut unknowns = Vec::new();
    if event.value_hash.is_none() {
        unknowns.push(
            "Observed value was not captured; validator can only assert event shape.".to_string(),
        );
    }
    if event.offset.is_none() && event.register.is_none() {
        unknowns.push("Event has no concrete address or register target.".to_string());
    }
    unknowns
}

fn taint_unknowns(mark: &TaintMark) -> Vec<String> {
    let mut unknowns = Vec::new();
    if mark.originating_op.is_none() {
        unknowns.push("Taint mark lacks an originating p-code op.".to_string());
    }
    if mark.indirect_read || mark.indirect_write {
        unknowns.push("Indirect taint depends on pointer/value replay fidelity.".to_string());
    }
    unknowns
}

fn event_requires_obligation(kind: &TraceEventKind, include_read_obligations: bool) -> bool {
    match kind {
        TraceEventKind::MemoryRead | TraceEventKind::RegisterRead => include_read_obligations,
        TraceEventKind::ThreadStep | TraceEventKind::PcodeStep => false,
        TraceEventKind::MemoryWrite
        | TraceEventKind::RegisterWrite
        | TraceEventKind::Syscall
        | TraceEventKind::Network
        | TraceEventKind::BranchDecision
        | TraceEventKind::Breakpoint => true,
    }
}

fn event_kind_label(kind: &TraceEventKind) -> &'static str {
    match kind {
        TraceEventKind::ThreadStep => "ThreadStep",
        TraceEventKind::PcodeStep => "PcodeStep",
        TraceEventKind::MemoryRead => "MemoryRead",
        TraceEventKind::MemoryWrite => "MemoryWrite",
        TraceEventKind::RegisterRead => "RegisterRead",
        TraceEventKind::RegisterWrite => "RegisterWrite",
        TraceEventKind::Syscall => "Syscall",
        TraceEventKind::Network => "Network",
        TraceEventKind::BranchDecision => "BranchDecision",
        TraceEventKind::Breakpoint => "Breakpoint",
    }
}

fn event_description(event: &TraceEventFact) -> String {
    format!(
        "{} event {} at {} with size {}, register {}, p-code op {}, and value hash {}.",
        event_kind_label(&event.kind),
        event.event_id,
        event_target(event),
        event
            .size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        event.register.as_deref().unwrap_or("none"),
        event.pcode_op.as_deref().unwrap_or("none"),
        event.value_hash.as_deref().unwrap_or("none"),
    )
}

fn event_replay_rule(event: &TraceEventFact) -> String {
    format!(
        "Replay {} at sequence {} in snapshot {}; target={}, size={}, value_hash={}, pcode_op={}.",
        event_kind_label(&event.kind),
        event.sequence,
        event.snapshot_id,
        event_target(event),
        event
            .size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        event.value_hash.as_deref().unwrap_or("none"),
        event.pcode_op.as_deref().unwrap_or("none"),
    )
}

fn event_target(event: &TraceEventFact) -> String {
    if let (Some(space), Some(offset)) = (&event.address_space, &event.offset) {
        format!("{space}:{offset}")
    } else if let Some(register) = &event.register {
        format!("register:{register}")
    } else {
        event.event_id.clone()
    }
}

fn taint_target(mark: &TaintMark) -> String {
    format!("{}:{}", mark.address_space, mark.offset)
}

fn taint_obligation(mark: &TaintMark) -> String {
    format!(
        "Preserve input influence labels [{}] over {} bytes at {}; originating_op={}, indirect_read={}, indirect_write={}.",
        mark.labels.join(", "),
        mark.size,
        taint_target(mark),
        mark.originating_op.as_deref().unwrap_or("unknown"),
        mark.indirect_read,
        mark.indirect_write,
    )
}

fn taint_validator_rule(mark: &TaintMark) -> String {
    format!(
        "During replay of snapshot {}{}, assert taint labels [{}] remain present at {} for {} bytes.",
        mark.snapshot_id,
        mark.event_id
            .as_ref()
            .map(|event_id| format!(" after event {event_id}"))
            .unwrap_or_default(),
        mark.labels.join(", "),
        taint_target(mark),
        mark.size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering::{
        compile_program_analysis_run_in_memory, BinaryArtifact, RuntimeTrace, TaintMark,
        TraceEventFact, TraceScheduleForm, TraceScheduleSource, TraceScheduleSpec,
        TraceSnapshotFact,
    };

    fn trace_analysis() -> ProgramAnalysisOutput {
        let artifact = BinaryArtifact {
            artifact_id: "artifact:sha256:abc".to_string(),
            sha256: "abc".to_string(),
            format: "ELF".to_string(),
            arch: "x86_64".to_string(),
            endian: "little".to_string(),
            entrypoints: vec!["0x401000".to_string()],
            load_base: Some("0x400000".to_string()),
            evidence_ids: vec!["e:artifact".to_string()],
        };
        let mut input = super::super::ProgramAnalysisInput::new("Travis-Gilbert", artifact);
        input.runtime_traces.push(RuntimeTrace {
            trace_id: "trace:login".to_string(),
            language_id: Some("x86:LE:64:default".to_string()),
            compiler_spec_id: Some("gcc".to_string()),
            emulator_cache_version: Some("42".to_string()),
            capture_source: "ghidra-trace-oracle".to_string(),
            evidence_ids: vec!["e:trace".to_string()],
        });
        input.trace_snapshots.push(TraceSnapshotFact {
            snapshot_id: "snapshot:login:0".to_string(),
            trace_id: "trace:login".to_string(),
            snap: 0,
            description: "entry".to_string(),
            real_time_ms: Some(100),
            event_thread_id: Some("thread:1".to_string()),
            schedule: TraceScheduleSpec {
                schedule: "0:3.1".to_string(),
                snap: 0,
                instruction_steps: 3,
                pcode_steps: 1,
                source: TraceScheduleSource::Record,
                form: TraceScheduleForm::SnapAnyStepsOps,
            },
            version: 1,
            forked: false,
            stale: false,
            evidence_ids: vec!["e:snapshot".to_string()],
        });
        input.trace_events.push(TraceEventFact {
            event_id: "event:login:branch".to_string(),
            trace_id: "trace:login".to_string(),
            snapshot_id: "snapshot:login:0".to_string(),
            sequence: 3,
            thread_id: Some("thread:1".to_string()),
            kind: TraceEventKind::BranchDecision,
            address_space: Some("ram".to_string()),
            offset: Some("0x401020".to_string()),
            size: Some(1),
            register: None,
            value_hash: Some("sha256:taken".to_string()),
            pcode_op: Some("CBRANCH".to_string()),
            evidence_ids: vec!["e:event".to_string()],
        });
        input.taint_marks.push(TaintMark {
            taint_id: "taint:login:password".to_string(),
            trace_id: "trace:login".to_string(),
            snapshot_id: "snapshot:login:0".to_string(),
            event_id: Some("event:login:branch".to_string()),
            address_space: "ram".to_string(),
            offset: "0x7fffffffe000".to_string(),
            size: 16,
            labels: vec!["password".to_string(), "http.body".to_string()],
            originating_op: Some("ram:0x401020:3".to_string()),
            indirect_read: true,
            indirect_write: false,
            evidence_ids: vec!["e:taint".to_string()],
        });
        compile_program_analysis_run_in_memory(input)
    }

    #[test]
    fn trace_to_engineering_input_emits_obligations_and_validators() {
        let analysis = trace_analysis();
        let input = program_analysis_trace_to_engineering_input(
            &analysis,
            TraceToEngineeringOptions::new("repo:login"),
        );

        assert_eq!(input.tenant_id, "Travis-Gilbert");
        assert_eq!(input.repo_id, "repo:login");
        assert_eq!(input.compiler_version, TRACE_ENGINEERING_COMPILER_VERSION);
        assert!(input
            .architecture_inputs
            .iter()
            .any(|item| item.component_name == "runtime-trace-replay"));
        assert!(input
            .behavior_inputs
            .iter()
            .any(|item| item.behavior_name == "Replay trace:login"));
        assert!(input
            .implementation_obligation_inputs
            .iter()
            .any(|item| item.obligation.contains("Preserve input influence labels")));
        assert!(input
            .validator_inputs
            .iter()
            .any(|item| item.validator_kind == "trace-schedule-replay"));
        assert!(input
            .validator_inputs
            .iter()
            .any(|item| item.validator_kind == "trace-event-replay"));
        assert!(input
            .validator_inputs
            .iter()
            .any(|item| item.validator_kind == "taint-presence"));
        assert!(input
            .evidence_sources
            .iter()
            .any(|source| source.authority.role == "derived_fact"));
    }

    #[test]
    fn trace_to_engineering_compile_writes_agent_facing_graph_nodes() {
        let analysis = trace_analysis();
        let output = compile_program_analysis_trace_engineering_in_memory(
            &analysis,
            TraceToEngineeringOptions::new("repo:login"),
        );

        assert!(!output.behavior_specs.is_empty());
        assert!(!output.implementation_obligations.is_empty());
        assert!(output
            .validator_specs
            .iter()
            .any(|validator| validator.validator_kind == "taint-presence"));
        assert!(output
            .graph_nodes
            .iter()
            .any(|node| node.labels.contains(&"EngineeringValidator".to_string())));
        assert!(!output.artifact_hash.is_empty());
    }
}
