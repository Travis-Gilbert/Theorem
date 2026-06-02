import Foundation

/// The source of runs for the Runs surface. The UI reads runs through this
/// protocol so the data source is swappable: today a recorded sample, later a
/// remote store backed by the Rust runtime's event log
/// (`theorem-harness-runtime`), with no view change.
///
/// The runtime persists each run as a `HarnessRun`-labelled node plus an ordered
/// list of `HarnessEvent` nodes in a GraphStore. A `RemoteHarnessRunStore` will
/// decode that contract once the runtime exposes an HTTP/SDK surface (spec
/// Part 7). The seam exists now so that wiring is a one-line change.
public protocol HarnessRunStore: Sendable {
    /// All runs the store knows about, most recent first.
    func runs() async throws -> [HarnessRun]
}

/// The default source: the recorded reference run from the parity corpus. Honest
/// about being a single recorded run rather than a live feed.
public struct SampleRunStore: HarnessRunStore {
    public init() {}

    public func runs() async throws -> [HarnessRun] {
        [SampleRun.fullLifecycle]
    }
}
