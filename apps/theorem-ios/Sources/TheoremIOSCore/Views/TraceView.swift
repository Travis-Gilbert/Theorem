import SwiftUI

/// The trace surface (harness UI spec, Part 5): the run's CognitiveTrace as the
/// user's own legible, exportable record. Observable and explicit (never
/// overpromising hidden chain-of-thought): the event stream with what each
/// transition did, plus a real export. The framing is ownership: "you own the
/// record of how your agents worked, and you can export it."
///
/// The export is a working action, not a decorative button: it writes a real
/// `trace.jsonl` of the run's events and hands it to the share sheet. Additional
/// formats (OTel spans, provenance JSON-LD, SFT examples) arrive when the runtime
/// persists the full trace; they are named honestly, not offered as dead buttons.
struct TraceView: View {
    let run: HarnessRun
    var theme: TheoremTheme

    @State private var traceFile: URL?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                header
                exportSection
                traceSection
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(theme.field.ignoresSafeArea())
        .onAppear { traceFile = writeTrace() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("TRACE")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
            Text("How the run worked")
                .font(TheoremFonts.display(size: 24, relativeTo: .title))
                .foregroundStyle(theme.ink)
            Text("Your own record of how your agents worked, yours to export. Observable and explicit, never hidden chain-of-thought.")
                .font(TheoremFonts.body(size: 14)).foregroundStyle(theme.textSecondary)
                .lineSpacing(3)
        }
    }

    private var exportSection: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text("EXPORT")
                .font(TheoremFonts.label(size: 9)).tracking(0.8)
                .foregroundStyle(theme.textMuted)
            if let traceFile {
                ShareLink(item: traceFile) {
                    Label("Export trace.jsonl", systemImage: "square.and.arrow.up")
                        .font(TheoremFonts.label(size: 13))
                        .foregroundStyle(theme.field)
                        .padding(.horizontal, 14).padding(.vertical, 10)
                        .background(theme.ink, in: Capsule())
                }
                .buttonStyle(.plain)
            }
            Text("Also exports with the full runtime: OpenTelemetry spans, provenance JSON-LD, context artifacts, SFT and preference examples, context-rank labels, graph packs.")
                .font(TheoremFonts.body(size: 11)).foregroundStyle(theme.textMuted)
                .lineSpacing(2)
        }
        .padding(13)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.55), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }

    private var traceSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("COGNITIVE TRACE")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
                .padding(.bottom, 12)
            ForEach(run.events) { event in
                traceRow(event)
            }
        }
    }

    private func traceRow(_ event: HarnessRunEvent) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(event.type)
                    .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.ink)
                Spacer(minLength: 8)
                Text(event.stateHashAfter.prefix(8))
                    .font(TheoremFonts.mono(size: 10)).foregroundStyle(theme.textMuted)
            }
            Text(traceDescription(event))
                .font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 9)
        .overlay(alignment: .bottom) {
            Rectangle().fill(theme.hairline).frame(height: 1)
        }
    }

    /// An honest, data-derived one-line description per event. Where the run
    /// carries real detail (ledger, outcome), the description uses it; otherwise
    /// it names the phase. No invented specifics.
    private func traceDescription(_ event: HarnessRunEvent) -> String {
        switch event.type {
        case "RUN.CREATED": "Run opened. Actor \(run.actor)."
        case "HOST.OBSERVED": "Host observed: repo, branch, commit."
        case "TASK.RESOLVED": "Task signature resolved."
        case "PROFILE.SELECTED": "Profile selected."
        case "TOOLKIT.COMPILED": "Per-task toolkit compiled with permission reasons."
        case "MAPS.LOADED": "Orientation maps loaded."
        case "CONTEXT.PLANNED": "Context planned against the token budget."
        case "CONTEXT.PACKED":
            run.ledger.map { "Context packed: \($0.includedAtoms) atoms, \($0.capsuleTokens) of \($0.budgetTokens) tokens, \($0.savedTokens) saved." }
                ?? "Context packed within budget."
        case "CONTEXT.INJECTED": "Compiled context injected into the participant."
        case "AGENT.ACTING": "Participant acting on the task."
        case "OUTCOME.RECORDED":
            run.outcome.map { "Outcome recorded: \($0.accepted ? "accepted" : "rejected"), \($0.filesChanged.count) file(s) changed." }
                ?? "Outcome recorded."
        case "LEARNING.PROPOSED": "Learning proposed (review-gated, not applied)."
        case "REVIEW.QUEUED": "Queued for review."
        case "FEDERATION.SIGNAL_PREPARED": "Federation signal prepared: consented, content-free buckets."
        case "RUN.CLOSED": "Run closed. Final state hash recorded."
        default: event.statusLabel.lowercased().capitalized + "."
        }
    }

    private func writeTrace() -> URL? {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(run.runID)-trace.jsonl")
        do {
            try run.traceJSONL().write(to: url, atomically: true, encoding: .utf8)
            return url
        } catch {
            return nil
        }
    }
}

#Preview {
    NavigationStack {
        TraceView(run: SampleRun.fullLifecycle, theme: .defaultPalette)
    }
}
