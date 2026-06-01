import SwiftUI

/// The room conversation, rendered INSIDE the expanded Dynamic Island. The island
/// is the single chrome surface: tapping the pill morphs it into this (the ask,
/// participant presence, recent contributions), the same way tapping a node
/// morphs it into the node dossier. The room is no longer a slab below the graph
/// — it is the search box changing shape and context.
///
/// Renders from the CommonplaceRoom model (the room data), reusing only the
/// committed/stable API. The AI-contribution body decodes via ScrambleText.
struct IslandRoomView: View {
    let room: CommonplaceRoom
    var theme: TheoremTheme
    var onClose: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            Text(room.ask)
                .font(TheoremFonts.body(size: 15, relativeTo: .callout))
                .foregroundStyle(theme.ink)
                .lineLimit(2)
            if let creditPreview {
                roomCreditStrip(creditPreview)
            }
            presence
            Divider().overlay(theme.hairline)
            log
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Text("ROOM")
                .font(TheoremFonts.label(size: 10)).tracking(1.0)
                .foregroundStyle(theme.textMuted)
            Spacer()
            Text("\(room.engagedParticipants.count)/\(room.participants.count)")
                .font(TheoremFonts.mono(size: 11)).monospacedDigit()
                .foregroundStyle(theme.textMuted)
            Button(action: onClose) {
                Image(systemName: "xmark").font(.system(size: 12, weight: .bold))
                    .foregroundStyle(theme.textMuted)
            }
            .buttonStyle(.plain)
        }
    }

    private var presence: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(room.participants) { participant in
                    HStack(spacing: 5) {
                        Circle().fill(statusColor(participant.status)).frame(width: 7, height: 7)
                        Text(participant.shortName)
                            .font(TheoremFonts.mono(size: 11)).foregroundStyle(theme.ink)
                        Text(participant.status.rawValue)
                            .font(TheoremFonts.label(size: 9)).foregroundStyle(theme.textMuted)
                    }
                    .padding(.horizontal, 8).frame(height: 26)
                    .background(theme.chrome, in: Capsule())
                }
            }
        }
        .scrollClipDisabled()
    }

    private var creditPreview: CommonplaceCreditEstimate? {
        guard let registry = room.registry else { return nil }
        let routePlan = room.routePlan ?? CommonplaceRouter().plan(query: room.ask, registry: registry)
        let toolBudget = CommonplaceToolUseBudget.preview(for: room.ask, features: routePlan.features)
        return CommonplaceCreditEstimator().estimate(
            routePlan: routePlan,
            registry: registry,
            toolBudget: toolBudget
        )
    }

    private func roomCreditStrip(_ estimate: CommonplaceCreditEstimate) -> some View {
        HStack(spacing: 8) {
            Image(systemName: estimate.requiresConfirmation ? "exclamationmark.triangle.fill" : "creditcard")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(estimate.requiresConfirmation ? theme.signal : theme.blueprintInk)
                .frame(width: 16)
            Text(estimate.creditRangeLabel)
                .font(TheoremFonts.mono(size: 11))
                .foregroundStyle(theme.ink)
                .monospacedDigit()
            Spacer(minLength: 8)
            Text(estimate.requiresConfirmation ? "CONFIRM" : "PREVIEW")
                .font(TheoremFonts.label(size: 9))
                .tracking(0.7)
                .foregroundStyle(theme.textMuted)
        }
        .padding(.horizontal, 9)
        .frame(height: 28)
        .background(theme.chrome.opacity(0.72), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(theme.hairline, lineWidth: 1)
        )
    }

    private var log: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(room.contributions.suffix(4)) { contribution in
                    contributionRow(contribution)
                }
            }
        }
        .frame(maxHeight: 150)
    }

    private func contributionRow(_ contribution: CommonplaceContribution) -> some View {
        HStack(alignment: .top, spacing: 9) {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(authorColor(contribution))
                .frame(width: 3)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(authorLabel(contribution))
                        .font(TheoremFonts.label(size: 10)).tracking(0.6)
                        .foregroundStyle(theme.ink).lineLimit(1)
                    Text(contribution.state.rawValue)
                        .font(TheoremFonts.mono(size: 9)).foregroundStyle(theme.textMuted).lineLimit(1)
                }
                Text(contribution.summary)
                    .font(TheoremFonts.body(size: 12, relativeTo: .caption))
                    .foregroundStyle(theme.ink).lineLimit(1)
                ScrambleText(text: contribution.body, size: 11, color: theme.textMuted)
                    .lineLimit(2).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 4)
    }

    private func authorLabel(_ contribution: CommonplaceContribution) -> String {
        switch contribution.authorKind {
        case .human: "You"
        case .participant: room.participant(for: contribution.authorID)?.displayName ?? "Participant"
        case .substrate: "Substrate"
        }
    }

    private func authorColor(_ contribution: CommonplaceContribution) -> Color {
        switch contribution.authorKind {
        case .human:
            theme.ink
        case .participant:
            room.participant(for: contribution.authorID)?.status == .contributing ? theme.signal : theme.blueprintInk
        case .substrate:
            theme.ruleStrong
        }
    }

    private func statusColor(_ status: CommonplaceParticipantStatus) -> Color {
        switch status {
        case .thinking: theme.blueprintInk
        case .contributing: theme.signal
        case .idle: theme.pebble
        }
    }
}
