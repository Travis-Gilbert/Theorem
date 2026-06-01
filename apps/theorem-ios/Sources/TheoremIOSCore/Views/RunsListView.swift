import SwiftUI

/// The Runs surface (harness UI spec, Part 2 entry: "open a run, current or past").
/// Lists recorded runs and opens the run-detail surface on tap.
///
/// Today it lists the recorded reference run from the parity corpus
/// (`SampleRun.fullLifecycle`). When the runtime crate persists live events, live
/// runs appear here alongside it: same row, same detail view, no code change. The
/// list is honest about its single recorded run rather than padded with fakes.
struct RunsListView: View {
    var theme: TheoremTheme

    @State private var path: [String] = []

    private let runs: [HarnessRun] = [SampleRun.fullLifecycle]

    var body: some View {
        NavigationStack(path: $path) {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    header
                    ForEach(runs) { run in
                        NavigationLink(value: run.runID) {
                            runRow(run)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(theme.field.ignoresSafeArea())
            .navigationDestination(for: String.self) { runID in
                if let run = runs.first(where: { $0.runID == runID }) {
                    RunDetailView(run: run, theme: theme)
                }
            }
        }
        .task {
            // -runDetail 1 opens the first recorded run's detail directly
            // (deep-link + screenshot capture), consistent with the app's
            // -patent / -autoSearch launch arguments.
            if UserDefaults.standard.bool(forKey: "runDetail"), let first = runs.first {
                path = [first.runID]
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("RUNS")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
            Text("Recorded runs")
                .font(TheoremFonts.display(size: 28, relativeTo: .title))
                .foregroundStyle(theme.ink)
            Text("Each run is a governed state machine. Open one to inspect its lifecycle, evidence, cost, and outcome.")
                .font(TheoremFonts.body(size: 14)).foregroundStyle(theme.textSecondary)
                .lineSpacing(3)
        }
        .padding(.bottom, 2)
    }

    private func runRow(_ run: HarnessRun) -> some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(run.task)
                    .font(TheoremFonts.body(size: 15).weight(.medium))
                    .foregroundStyle(theme.ink).lineLimit(1)
                HStack(spacing: 8) {
                    Text(run.runID)
                        .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.textMuted)
                    Text("·").foregroundStyle(theme.pebble)
                    Text("\(run.events.count) events")
                        .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.textMuted)
                }
            }
            Spacer(minLength: 8)
            Text(run.status.replacingOccurrences(of: "_", with: " ").uppercased())
                .font(TheoremFonts.label(size: 9)).tracking(0.6)
                .foregroundStyle(run.isTerminal ? theme.field : theme.ink)
                .padding(.horizontal, 9).padding(.vertical, 4)
                .background(run.isTerminal ? theme.ink : theme.chrome, in: Capsule())
            Image(systemName: "chevron.right")
                .font(.system(size: 12, weight: .semibold)).foregroundStyle(theme.pebble)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }
}

#Preview {
    RunsListView(theme: .defaultPalette)
}
