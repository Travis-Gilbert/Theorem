import Foundation

/// A `ParticipantStore` backed by the harness runtime's HTTP presence surface
/// (`GET /harness/rooms/{room}/presence?tenant={slug}`, theorem-harness-server).
/// Joins the known roster (identity + access model, Part 7) with live presence
/// status, so the Participants surface shows who is actually active in the room
/// without inventing activity. Swapping this in for `SampleParticipantStore`
/// makes the surface render live presence with no view change.
///
/// Honest about the dependency: live status only appears once the runtime has
/// presence heartbeats for the tenant. With an empty presence feed every roster
/// member reads idle, which is the truth, not a fake. Reuses
/// `HarnessRunStoreError` so transport/status/decoding failures surface the same
/// way as the runs path.
public struct RemoteParticipantStore: ParticipantStore {
    public let baseURL: URL
    public let roomID: String
    public let tenantSlug: String
    private let roster: [Participant]
    private let session: URLSession

    public init(
        baseURL: URL,
        roomID: String = "default",
        tenantSlug: String = "default",
        roster: [Participant] = SampleRoster.participants,
        session: URLSession = .shared
    ) {
        self.baseURL = baseURL
        self.roomID = roomID
        self.tenantSlug = tenantSlug
        self.roster = roster
        self.session = session
    }

    public func participants() async throws -> [Participant] {
        let path = "harness/rooms/\(roomID)/presence"
        guard
            var components = URLComponents(
                url: baseURL.appendingPathComponent(path),
                resolvingAgainstBaseURL: false
            )
        else {
            throw HarnessRunStoreError.transport("Bad presence URL")
        }
        components.queryItems = [URLQueryItem(name: "tenant", value: tenantSlug)]
        guard let url = components.url else {
            throw HarnessRunStoreError.transport("Bad presence URL")
        }

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            throw HarnessRunStoreError.transport("No HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw HarnessRunStoreError.status(http.statusCode)
        }
        let decoded: PresenceListResponse
        do {
            decoded = try JSONDecoder().decode(PresenceListResponse.self, from: data)
        } catch {
            throw HarnessRunStoreError.decoding(String(describing: error))
        }
        return ParticipantPresenceJoin.merge(roster: roster, presence: decoded.presence)
    }
}
