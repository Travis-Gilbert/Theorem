import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// The coordination Room surface (docs/plans/coordination-room-push): the app as
/// a member of the same room the agents are in. One text field, one send button,
/// and the room's live message feed. The button is the whole interaction:
///   - TAP  -> passive: leave a note. Streamed to watchers; agents see it next
///             time they are live and call `mentions`.
///   - HOLD -> wake: queue the agents. The button fills as you hold; on release
///             the spawn-listener wakes the addressed agents (or all room agents).
/// A single boolean (`delivery`) is the whole difference; the fill + haptic make
/// "I queued them" legible against "I left a note".
struct RoomComposerView: View {
    var theme: TheoremTheme
    /// The room channel. Defaults to the offline sample; `-remote <url>` swaps in
    /// the harness-server-backed channel with no view change.
    var channel: RoomChannel = SampleRoomChannel()

    @State private var draft: String = ""
    @State private var messages: [RoomMessage] = []
    @State private var holdProgress: CGFloat = 0
    @State private var didWake = false
    @State private var sendError: String?
    @State private var streamState: StreamState = .connecting

    private let holdDuration: Double = 0.5

    enum StreamState: Equatable {
        case connecting
        case live
        case closed
        case failed(String)
    }

    private var trimmedDraft: String {
        draft.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(theme.hairline)
            feed
            if let sendError {
                Text(sendError)
                    .font(TheoremFonts.mono(size: 11))
                    .foregroundStyle(theme.surface)
                    .padding(.horizontal, 12).padding(.vertical, 7)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.ringMatch)
                    .onTapGesture { self.sendError = nil }
            }
            composer
        }
        .background(theme.field.ignoresSafeArea())
        .task { await subscribe() }
    }

    // MARK: Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Text("ROOM")
                    .font(TheoremFonts.label(size: 10)).tracking(0.9)
                    .foregroundStyle(theme.textMuted)
                streamBadge
            }
            Text("The shared room")
                .font(TheoremFonts.display(size: 28, relativeTo: .title))
                .foregroundStyle(theme.ink)
            Text("Tap to leave a note. Hold to wake the agents. You and the agents are members of the same room.")
                .font(TheoremFonts.body(size: 14)).foregroundStyle(theme.textSecondary)
                .lineSpacing(3)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(20)
    }

    @ViewBuilder private var streamBadge: some View {
        let (label, live): (String, Bool) = {
            switch streamState {
            case .connecting: ("connecting", false)
            case .live: ("live", true)
            case .closed: ("offline", false)
            case .failed: ("disconnected", false)
            }
        }()
        HStack(spacing: 5) {
            Circle()
                .fill(live ? theme.signal : theme.pebble)
                .frame(width: 6, height: 6)
            Text(label)
                .font(TheoremFonts.label(size: 9)).tracking(0.6)
                .foregroundStyle(theme.textMuted)
        }
        .padding(.horizontal, 7).padding(.vertical, 3)
        .background(theme.chrome.opacity(0.6), in: Capsule())
    }

    // MARK: Feed

    private var feed: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if messages.isEmpty {
                        Text("No messages yet. Say something to the room.")
                            .font(TheoremFonts.body(size: 13)).foregroundStyle(theme.textMuted)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.top, 12)
                    } else {
                        ForEach(messages) { message in
                            messageRow(message).id(message.id)
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 14)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: messages.count) {
                if let last = messages.last {
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
        }
    }

    private func messageRow(_ message: RoomMessage) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 7) {
                Text(message.author.isEmpty ? "unknown" : message.author)
                    .font(TheoremFonts.mono(size: 12).weight(.medium))
                    .foregroundStyle(theme.ink)
                deliveryBadge(message.delivery)
                Spacer(minLength: 6)
            }
            Text(message.body)
                .font(TheoremFonts.body(size: 14))
                .foregroundStyle(theme.textPrimary)
                .frame(maxWidth: .infinity, alignment: .leading)
            if !message.mentions.isEmpty {
                Text(message.mentions.map { "@\($0)" }.joined(separator: " "))
                    .font(TheoremFonts.mono(size: 11))
                    .foregroundStyle(theme.textMuted)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.chrome.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(message.delivery == .wake ? theme.signal.opacity(0.6) : theme.hairline, lineWidth: 1)
        )
    }

    private func deliveryBadge(_ delivery: MessageDelivery) -> some View {
        HStack(spacing: 4) {
            Image(systemName: delivery == .wake ? "bolt.fill" : "paperplane")
                .font(.system(size: 8, weight: .bold))
            Text(delivery == .wake ? "WAKE" : "NOTE")
                .font(TheoremFonts.label(size: 8)).tracking(0.6)
        }
        .foregroundStyle(delivery == .wake ? theme.field : theme.textMuted)
        .padding(.horizontal, 6).padding(.vertical, 2)
        .background(delivery == .wake ? theme.signal : theme.chrome, in: Capsule())
    }

    // MARK: Composer (the single send button)

    private var composer: some View {
        HStack(spacing: 10) {
            TextField("Message the room", text: $draft, axis: .vertical)
                .textFieldStyle(.plain)
                .font(TheoremFonts.body(size: 15))
                .foregroundStyle(theme.ink)
                .lineLimit(1...4)
                .padding(.horizontal, 14).padding(.vertical, 10)
                .background(theme.surface, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.hairline, lineWidth: 1)
                )

            sendButton
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(theme.field)
    }

    private var sendButton: some View {
        let pressing = holdProgress > 0.02
        let armed = holdProgress > 0.55
        return ZStack {
            Capsule().fill(theme.chrome)
            // The fill grows from the leading edge as you hold; full = armed wake.
            Capsule()
                .fill(theme.signal)
                .scaleEffect(x: holdProgress, y: 1, anchor: .leading)
            HStack(spacing: 6) {
                Image(systemName: armed ? "bolt.fill" : "paperplane.fill")
                    .font(.system(size: 13, weight: .bold))
                Text(armed ? "Wake" : "Send")
                    .font(TheoremFonts.label(size: 12)).tracking(0.4)
            }
            .foregroundStyle(pressing ? theme.field : theme.ink)
        }
        .frame(width: 96, height: 44)
        .clipShape(Capsule())
        .contentShape(Capsule())
        .opacity(trimmedDraft.isEmpty ? 0.4 : 1)
        .onLongPressGesture(
            minimumDuration: holdDuration,
            maximumDistance: 60,
            pressing: { isPressing in
                if isPressing {
                    didWake = false
                    withAnimation(.easeInOut(duration: holdDuration)) { holdProgress = 1 }
                } else {
                    withAnimation(.easeOut(duration: 0.2)) { holdProgress = 0 }
                }
            },
            perform: {
                // Held long enough: this is a wake, not a tap.
                didWake = true
                wakeHaptic()
                Task { await send(.wake) }
            }
        )
        .simultaneousGesture(
            TapGesture().onEnded {
                // A short tap leaves a note; ignore the tap that trails a long press.
                if !didWake { Task { await send(.passive) } }
            }
        )
        .disabled(trimmedDraft.isEmpty)
        .accessibilityLabel("Send to room")
        .accessibilityHint("Tap to leave a note. Touch and hold to wake the agents.")
    }

    // MARK: Actions

    @MainActor
    private func send(_ delivery: MessageDelivery) async {
        let body = trimmedDraft
        guard !body.isEmpty else { return }
        sendError = nil
        do {
            let message = try await channel.send(body, delivery: delivery)
            draft = ""
            appendUnique(message)
        } catch {
            sendError = Self.describe(error)
        }
        didWake = false
        withAnimation(.easeOut(duration: 0.2)) { holdProgress = 0 }
    }

    @MainActor
    private func subscribe() async {
        streamState = .connecting
        do {
            for try await message in channel.stream() {
                if streamState != .live { streamState = .live }
                appendUnique(message)
            }
            // A finished stream is honest: the sample channel has no live feed, and
            // a closed remote socket is "offline", not an error.
            if case .failed = streamState {} else { streamState = .closed }
        } catch {
            streamState = .failed(Self.describe(error))
        }
    }

    /// Append, or replace in place when the id is already present. The POST
    /// response and the SSE echo deliver the same message; dedupe by id avoids
    /// showing it twice.
    private func appendUnique(_ message: RoomMessage) {
        if let index = messages.firstIndex(where: { $0.id == message.id }) {
            messages[index] = message
        } else {
            messages.append(message)
        }
    }

    private func wakeHaptic() {
        #if canImport(UIKit)
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        #endif
    }

    private static func describe(_ error: Error) -> String {
        switch error {
        case HarnessRunStoreError.status(let code): "Server returned \(code)."
        case HarnessRunStoreError.transport(let message): message
        case HarnessRunStoreError.decoding: "Couldn't read the room response."
        default: "Send failed."
        }
    }
}

#Preview {
    RoomComposerView(theme: .defaultPalette)
}
