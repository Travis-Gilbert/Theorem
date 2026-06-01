import Foundation

/// A `HarnessRunStore` backed by the harness runtime's HTTP surface (spec
/// Part 7). Fetches the run list and each run's event log, decoding the runtime's
/// JSON contract (`RunDetailResponse`) into the UI's `HarnessRun`. Swapping this
/// in for `SampleRunStore` makes every run surface render live runs with no view
/// change.
///
/// Honest about the dependency: this only works once the runtime exposes
/// `GET /harness/runs` and `GET /harness/runs/{id}`
/// (docs/plans/harness-rust-port/ios-transport-handoff.md). Until then the app
/// defaults to `SampleRunStore`.
public struct RemoteHarnessRunStore: HarnessRunStore {
    public let baseURL: URL
    private let session: URLSession

    public init(baseURL: URL, session: URLSession = .shared) {
        self.baseURL = baseURL
        self.session = session
    }

    public func runs() async throws -> [HarnessRun] {
        let list: RunsListResponse = try await get("harness/runs")
        var result: [HarnessRun] = []
        result.reserveCapacity(list.runs.count)
        for node in list.runs {
            let detail: RunDetailResponse = try await get("harness/runs/\(node.runID)")
            result.append(detail.toHarnessRun())
        }
        return result
    }

    /// Fetch and decode a single run (used by detail surfaces that load lazily).
    public func run(_ runID: String) async throws -> HarnessRun {
        let detail: RunDetailResponse = try await get("harness/runs/\(runID)")
        return detail.toHarnessRun()
    }

    private func get<T: Decodable>(_ path: String) async throws -> T {
        let url = baseURL.appendingPathComponent(path)
        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            throw HarnessRunStoreError.transport("No HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            throw HarnessRunStoreError.status(http.statusCode)
        }
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw HarnessRunStoreError.decoding(String(describing: error))
        }
    }
}

public enum HarnessRunStoreError: Error, Equatable, Sendable {
    case transport(String)
    case status(Int)
    case decoding(String)

    public var message: String {
        switch self {
        case .transport(let detail): "Network error: \(detail)"
        case .status(let code): "Server returned \(code)."
        case .decoding: "Couldn't read the run from the server."
        }
    }
}
