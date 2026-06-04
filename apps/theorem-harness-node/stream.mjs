// The consumer side of "runs that stream typed events": an async iterator over a
// run's events.
//
//   for await (const event of streamRun(harness, runId)) { ... }
//
// It polls the durable cursor (eventsJson, filtered by sequence) and yields each
// new typed event until the run reaches a terminal status. The transport is
// poll-based, matching the Rust SDK's resumable primitive `RunStream::events_since`;
// a napi ThreadsafeFunction push is a later optimization with the identical
// consumer shape, so callers of `streamRun` do not change when it lands.

const TERMINAL = new Set(["closed", "failed", "cancelled"]);

/**
 * Yield a run's events as they appear, until the run is terminal.
 *
 * @param harness a Harness instance (the napi binding)
 * @param runId the run to stream
 * @param opts.intervalMs poll interval between catch-up reads (default 25)
 * @param opts.maxPolls safety bound so a never-terminating run cannot hang the
 *        iterator forever (default 1000)
 */
export async function* streamRun(harness, runId, { intervalMs = 25, maxPolls = 1000 } = {}) {
  let cursor = 0;
  for (let poll = 0; poll < maxPolls; poll++) {
    const fresh = JSON.parse(harness.eventsJson(runId)).filter((event) => event.seq > cursor);
    for (const event of fresh) {
      cursor = event.seq;
      yield event;
    }
    if (TERMINAL.has(harness.runStatus(runId))) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}
