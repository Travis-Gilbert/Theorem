import SwiftUI

/// The participant presence surface (harness UI spec, Part 4: "a team working,
/// not a menu"). Shows the roster and how each participant connects (the three
/// access modes), addressed collectively through Ask. It reads through a
/// `ParticipantStore`, so the same surface renders the recorded roster offline
/// and live room presence when pointed at a runtime (`-remote`), with no view
/// change.
///
/// Honest status, not decoration: with no active presence the team is idle and
/// the banner says so; when the runtime reports active actors the banner and the
/// rows go live. A transport failure surfaces the error rather than silently
/// falling back to a faked-active roster.
struct ParticipantPresenceView: View {
    var theme: TheoremTheme
    /// The data source. Defaults to the recorded roster; swap for a runtime-backed
    /// store (`RemoteParticipantStore`) to render live presence.
    var store: ParticipantStore = SampleParticipantStore()

    @State private var participants: [Participant] = []
    @State private var state: LoadState = .loading

    enum LoadState: Equatable {
        case loading
        case loaded
        case failed(String)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                content
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(theme.field.ignoresSafeArea())
        .task { await load() }
    }

    @ViewBuilder private var content: some View {
        switch state {
        case .loading:
            HStack(spacing: 9) {
                ProgressView().controlSize(.small).tint(theme.signal)
                Text("Reading room presence\u{2026}")
                    .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
            }
            .padding(.top, 4)
        case .failed(let message):
            honest(message)
        case .loaded:
            statusBanner
            VStack(spacing: 10) {
                ForEach(participants) { participant in
                    participantRow(participant)
                }
            }
        }
    }

    private func honest(_ message: String) -> some View {
        Text(message)
            .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 4)
    }

    @MainActor
    private func load() async {
        state = .loading
        do {
            participants = try await store.participants()
            state = .loaded
        } catch {
            let message = (error as? HarnessRunStoreError)?.message ?? "Couldn't read presence."
            state = .failed(message)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("PARTICIPANTS")
                .font(TheoremFonts.label(size: 10)).tracking(0.9)
                .foregroundStyle(theme.textMuted)
            Text("The team")
                .font(TheoremFonts.display(size: 28, relativeTo: .title))
                .foregroundStyle(theme.ink)
            Text("Addressed through Ask. The router engages them by capability; you don't pick a model. The exception is a brought agent, which you supplied and can address directly.")
                .font(TheoremFonts.body(size: 14)).foregroundStyle(theme.textSecondary)
                .lineSpacing(3)
        }
    }

    /// Reflects real presence: idle when no participant is engaged or
    /// contributing, live (with a count) when the runtime reports active actors.
    private var statusBanner: some View {
        let live = participants.filter { $0.status == .engaged || $0.status == .contributing }
        let isLive = !live.isEmpty
        return HStack(spacing: 9) {
            Image(systemName: isLive ? "dot.radiowaves.left.and.right" : "pause.circle")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(isLive ? theme.signal : theme.textMuted)
            Text(isLive
                ? "\(live.count) active in the room now."
                : "No active run. The team is idle; live status appears when a run engages them.")
                .font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textMuted)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private func participantRow(_ participant: Participant) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 9) {
                Circle().fill(statusColor(participant.status)).frame(width: 7, height: 7)
                Text(participant.name)
                    .font(TheoremFonts.body(size: 15).weight(.medium))
                    .foregroundStyle(theme.ink)
                Spacer(minLength: 8)
                tag(participant.kind.label)
                tag(participant.access.label)
            }
            Text(participant.note)
                .font(TheoremFonts.body(size: 12)).foregroundStyle(theme.textMuted)
                .lineSpacing(2)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(13)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
        )
    }

    private func tag(_ text: String) -> some View {
        Text(text)
            .font(TheoremFonts.label(size: 9))
            .foregroundStyle(theme.textMuted)
            .padding(.horizontal, 7).padding(.vertical, 3)
            .background(theme.chrome, in: Capsule())
    }

    private func statusColor(_ status: ParticipantStatus) -> Color {
        switch status {
        case .idle: theme.pebble
        case .engaged: theme.ruleStrong
        case .contributing: theme.signal
        case .unreachable: theme.hairline
        }
    }
}

#Preview {
    ParticipantPresenceView(theme: .defaultPalette)
}
