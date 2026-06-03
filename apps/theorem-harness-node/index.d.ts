// TypeScript declarations for the theorem-harness Node binding.
//
// A hand-written mirror of the napi `#[napi]` surface in src/lib.rs (camelCased
// from the Rust snake_case by napi). The @napi-rs/cli can auto-generate this from
// the annotations once wired (see README); until then this is the typed contract
// the plugin and any TS consumer import.

/** A harness bound to a durable RedCore graph store. */
export declare class Harness {
  /** Open a harness over a durable RedCore store at `dataDir` (AOF-backed, recovered on open). */
  constructor(dataDir: string)
  /** Start a run; returns the run id. */
  startRun(task: string, actor: string, idempotencyKey: string): string
  /** Cancel a run. */
  cancel(runId: string, reason: string, idempotencyKey: string): void
  /** All events for a run as a JSON array string. */
  eventsJson(runId: string): string
  /** Drain the text view of a run from a sequence cursor; returns the new text. */
  pollText(runId: string, afterSeq: number): string
  /** The current run status (`created`, `cancelled`, `closed`, ...) or `unknown`. */
  runStatus(runId: string): string
  /** Save a durable memory; returns the receipt as a JSON string. */
  remember(agentId: string, kind: string, title: string, content: string): string
  /** Recall memories matching `query`, as a JSON array string. */
  recall(agentId: string, query: string, limit: number): string
}
