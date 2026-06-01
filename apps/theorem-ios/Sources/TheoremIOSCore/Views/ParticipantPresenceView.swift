import SwiftUI

/// The participant presence surface (harness UI spec, Part 4: "a team working,
/// not a menu"). Shows the roster and how each participant connects (the three
/// access modes), addressed collectively through Ask. It is honest status, not
/// decoration: with no active run the team is idle, and the surface says so
/// rather than faking "thinking" activity. Live engaged / contributing status
/// arrives with the runtime's event stream.
struct ParticipantPresenceView: View {
    var theme: TheoremTheme

    private let participants: [Participant] = SampleRoster.participants

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                idleBanner
                VStack(spacing: 10) {
                    ForEach(participants) { participant in
                        participantRow(participant)
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(theme.field.ignoresSafeArea())
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

    private var idleBanner: some View {
        HStack(spacing: 9) {
            Image(systemName: "pause.circle")
                .font(.system(size: 13, weight: .semibold)).foregroundStyle(theme.textMuted)
            Text("No active run. The team is idle; live status appears when a run engages them.")
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
