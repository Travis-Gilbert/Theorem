import SwiftUI

struct CommonplaceRoomView: View {
    var room: CommonplaceRoom
    @Binding var projection: ProjectionID
    @Binding var selectedNodeID: String?
    var theme: TheoremTheme

    var body: some View {
        VStack(spacing: 10) {
            Spacer(minLength: 36)

            roomHeader

            ZStack(alignment: .bottom) {
                TheoremSceneView(
                    package: room.scene,
                    projection: projection,
                    selectedNodeID: $selectedNodeID,
                    theme: theme
                )

                RoomLogShelf(room: room, theme: theme)
                    .padding(.horizontal, 4)
                    .padding(.bottom, 6)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            Spacer(minLength: 104)
        }
        .padding(.horizontal, 14)
    }

    private var roomHeader: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Text(roomModeLabel)
                    .font(TheoremFonts.label(size: 10))
                    .tracking(1.0)
                    .foregroundStyle(theme.surface)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(theme.signal, in: RoundedRectangle(cornerRadius: 6, style: .continuous))

                Text(room.ask)
                    .font(TheoremFonts.body(size: 15, relativeTo: .callout))
                    .foregroundStyle(theme.ink)
                    .lineLimit(1)
                    .minimumScaleFactor(0.78)

                Spacer(minLength: 8)

                Text("\(room.engagedParticipants.count)/\(room.participants.count)")
                    .font(TheoremFonts.mono(size: 11))
                    .foregroundStyle(theme.textMuted)
                    .monospacedDigit()
            }

            RoomPresenceStrip(participants: room.participants, theme: theme)
        }
    }

    private var roomModeLabel: String {
        switch room.mode {
        case .ask:
            "ASK"
        case .addressBroughtAgent:
            "ADDRESS"
        }
    }
}

private struct RoomPresenceStrip: View {
    var participants: [CommonplaceParticipant]
    var theme: TheoremTheme

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 7) {
                ForEach(participants) { participant in
                    PresenceChip(participant: participant, theme: theme)
                }
            }
            .padding(.vertical, 1)
        }
        .scrollClipDisabled()
    }
}

private struct PresenceChip: View {
    var participant: CommonplaceParticipant
    var theme: TheoremTheme

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(statusColor)
                .frame(width: 7, height: 7)

            Text(participant.shortName)
                .font(TheoremFonts.mono(size: 11))
                .foregroundStyle(theme.ink)
                .lineLimit(1)

            Text(participant.status.rawValue)
                .font(TheoremFonts.label(size: 9))
                .foregroundStyle(theme.textMuted)
                .lineLimit(1)

            if participant.isDirectlyAddressable {
                Image(systemName: "person.badge.plus")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(theme.signal)
            }
        }
        .padding(.horizontal, 9)
        .frame(height: 30)
        .background(theme.chrome, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(theme.hairline, lineWidth: 1)
        )
    }

    private var statusColor: Color {
        switch participant.status {
        case .thinking:
            theme.blueprintInk
        case .contributing:
            theme.signal
        case .idle:
            theme.pebble
        }
    }
}

private struct RoomLogShelf: View {
    var room: CommonplaceRoom
    var theme: TheoremTheme

    private var visibleContributions: [CommonplaceContribution] {
        Array(room.contributions.suffix(4))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("ROOM")
                    .font(TheoremFonts.label(size: 10))
                    .tracking(1.1)
                    .foregroundStyle(theme.textMuted)
                Spacer()
                Text(room.updatedAt, style: .time)
                    .font(TheoremFonts.mono(size: 10))
                    .foregroundStyle(theme.textMuted)
            }

            ScrollView(showsIndicators: false) {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(visibleContributions) { contribution in
                        ContributionRow(
                            contribution: contribution,
                            participant: room.participant(for: contribution.authorID),
                            theme: theme
                        )
                    }
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, maxHeight: 172, alignment: .topLeading)
        .background(theme.chrome.opacity(0.94), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(theme.hairline, lineWidth: 1)
        )
    }
}

private struct ContributionRow: View {
    var contribution: CommonplaceContribution
    var participant: CommonplaceParticipant?
    var theme: TheoremTheme

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(authorColor)
                .frame(width: 3)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(authorLabel)
                        .font(TheoremFonts.label(size: 10))
                        .tracking(0.6)
                        .foregroundStyle(theme.ink)
                        .lineLimit(1)

                    Text(contribution.state.rawValue)
                        .font(TheoremFonts.mono(size: 9))
                        .foregroundStyle(theme.textMuted)
                        .lineLimit(1)
                }

                Text(contribution.summary)
                    .font(TheoremFonts.body(size: 12, relativeTo: .caption))
                    .foregroundStyle(theme.ink)
                    .lineLimit(1)

                Text(contribution.body)
                    .font(TheoremFonts.body(size: 11, relativeTo: .caption2))
                    .foregroundStyle(theme.textMuted)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 5)
        .padding(.horizontal, 6)
    }

    private var authorLabel: String {
        switch contribution.authorKind {
        case .human:
            "You"
        case .participant:
            participant?.displayName ?? "Participant"
        case .substrate:
            "Substrate"
        }
    }

    private var authorColor: Color {
        switch contribution.authorKind {
        case .human:
            theme.ink
        case .participant:
            participant?.status == .contributing ? theme.signal : theme.blueprintInk
        case .substrate:
            theme.ruleStrong
        }
    }
}
