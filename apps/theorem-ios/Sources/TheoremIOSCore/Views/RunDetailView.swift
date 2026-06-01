import SwiftUI

/// The run-detail surface (harness UI spec, Part 2: "the killer view"). It renders
/// a `HarnessRun` as the state machine it is, not a chat transcript: the enforced
/// lifecycle as a timeline (each transition with its status and content-addressed
/// hash), and the Evidence / Cost / Outcome rails (what the run read, what it
/// spent, what it changed and learned).
///
/// The data is a real run (today `SampleRun.fullLifecycle`, a recorded reference
/// run from the parity corpus; later a live `HarnessRun` from the runtime's event
/// log). The view is identical for both: it renders the run, whatever its source.
struct RunDetailView: View {
    let run: HarnessRun
    var theme: TheoremTheme

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                rails
                timeline
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(theme.field.ignoresSafeArea())
    }

    // MARK: Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("RUN")
                    .font(TheoremFonts.label(size: 10)).tracking(0.9)
                    .foregroundStyle(theme.textMuted)
                Spacer()
                statusChip
            }
            Text(run.task)
                .font(TheoremFonts.display(size: 24, relativeTo: .title))
                .foregroundStyle(theme.ink)
                .lineLimit(2)
            HStack(spacing: 8) {
                Text(run.runID)
                    .font(TheoremFonts.mono(size: 12)).foregroundStyle(theme.textMuted)
                Text("·").foregroundStyle(theme.pebble)
                Text(run.actor)
                    .font(TheoremFonts.mono(size: 12)).foregroundStyle(theme.textMuted)
            }
        }
    }

    private var statusChip: some View {
        Text(run.status.replacingOccurrences(of: "_", with: " ").uppercased())
            .font(TheoremFonts.label(size: 10)).tracking(0.7)
            .foregroundStyle(run.isTerminal ? theme.field : theme.ink)
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(run.isTerminal ? theme.ink : theme.chrome, in: Capsule())
    }

    // MARK: Rails (Evidence / Cost / Outcome)

    @ViewBuilder private var rails: some View {
        VStack(spacing: 10) {
            if let ledger = run.ledger {
                evidenceRail(ledger)
                costRail(ledger)
            }
            if let outcome = run.outcome {
                outcomeRail(outcome)
            }
        }
    }

    private func evidenceRail(_ ledger: HarnessRunLedger) -> some View {
        railCard(title: "EVIDENCE") {
            HStack(spacing: 6) {
                readout("\(ledger.includedAtoms)", "included")
                railDot
                readout("\(ledger.excludedAtoms)", "excluded")
                Spacer()
                Text(ledger.artifactID)
                    .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.textMuted)
                    .padding(.horizontal, 7).padding(.vertical, 3)
                    .background(theme.chrome, in: Capsule())
            }
        }
    }

    private func costRail(_ ledger: HarnessRunLedger) -> some View {
        railCard(title: "COST") {
            VStack(alignment: .leading, spacing: 7) {
                HStack(alignment: .firstTextBaseline) {
                    readout("\(ledger.capsuleTokens)", "of \(ledger.budgetTokens) tokens")
                    Spacer()
                    Text("\(ledger.savedTokens) saved")
                        .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.textMuted)
                }
                budgetBar(fraction: ledger.budgetFraction)
            }
        }
    }

    private func outcomeRail(_ outcome: HarnessRunOutcome) -> some View {
        railCard(title: "OUTCOME") {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    badge(outcome.accepted ? "accepted" : "rejected", on: outcome.accepted)
                    badge(outcome.testsPassed ? "tests passed" : "tests failed", on: outcome.testsPassed)
                    Spacer()
                }
                if !outcome.filesChanged.isEmpty {
                    HStack(spacing: 6) {
                        Text("CHANGED").font(TheoremFonts.label(size: 9)).tracking(0.7)
                            .foregroundStyle(theme.textMuted)
                        Text(outcome.filesChanged.joined(separator: ", "))
                            .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.ink)
                            .lineLimit(1)
                    }
                }
                ForEach(outcome.validators) { validator in
                    HStack(spacing: 6) {
                        Image(systemName: validator.passed ? "checkmark" : "xmark")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(validator.passed ? theme.ink : theme.signal)
                        Text(validator.id)
                            .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.textSecondary)
                        Text(validator.status)
                            .font(TheoremFonts.label(size: 10)).foregroundStyle(theme.textMuted)
                    }
                }
            }
        }
    }

    // MARK: Timeline (the run as a state machine)

    private var timeline: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("LIFECYCLE")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
                .padding(.bottom, 10)
            ForEach(Array(run.events.enumerated()), id: \.element.id) { index, event in
                timelineRow(event, isLast: index == run.events.count - 1)
            }
        }
    }

    private func timelineRow(_ event: HarnessRunEvent, isLast: Bool) -> some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(spacing: 0) {
                Circle()
                    .fill(isLast ? theme.signal : theme.ruleStrong)
                    .frame(width: 7, height: 7)
                    .padding(.top, 3)
                if !isLast {
                    Rectangle().fill(theme.hairline)
                        .frame(width: 1).frame(maxHeight: .infinity)
                }
            }
            .frame(width: 7)

            VStack(alignment: .leading, spacing: 2) {
                Text(event.statusLabel)
                    .font(TheoremFonts.label(size: 11)).tracking(0.5)
                    .foregroundStyle(theme.ink)
                Text(event.type)
                    .font(TheoremFonts.mono(size: 10)).foregroundStyle(theme.textMuted)
            }
            Spacer(minLength: 8)

            VStack(alignment: .trailing, spacing: 2) {
                Text(event.stateHashAfter.prefix(8))
                    .font(TheoremFonts.mono(size: 10)).foregroundStyle(theme.textMuted)
                    .padding(.horizontal, 7).padding(.vertical, 3)
                    .background(theme.chrome, in: Capsule())
                Text("#\(event.seq)")
                    .font(TheoremFonts.mono(size: 9)).foregroundStyle(theme.pebble)
            }
        }
        .padding(.bottom, isLast ? 0 : 14)
    }

    // MARK: Building blocks

    private func railCard<Content: View>(
        title: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(TheoremFonts.label(size: 9)).tracking(0.8)
                .foregroundStyle(theme.textMuted)
            content()
        }
        .padding(13)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.55), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }

    private func readout(_ value: String, _ label: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Text(value).font(TheoremFonts.mono(size: 15)).foregroundStyle(theme.ink)
            Text(label).font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textMuted)
        }
    }

    private var railDot: some View {
        Circle().fill(theme.pebble).frame(width: 3, height: 3)
    }

    private func badge(_ text: String, on: Bool) -> some View {
        Text(text)
            .font(TheoremFonts.label(size: 10))
            .foregroundStyle(on ? theme.field : theme.textMuted)
            .padding(.horizontal, 9).padding(.vertical, 4)
            .background(on ? theme.ink : theme.chrome, in: Capsule())
    }

    private func budgetBar(fraction: Double) -> some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(theme.pebble.opacity(0.5))
                Capsule().fill(theme.ink)
                    .frame(width: max(2, geo.size.width * fraction))
            }
        }
        .frame(height: 5)
    }
}

#Preview {
    RunDetailView(run: SampleRun.fullLifecycle, theme: .defaultPalette)
}
