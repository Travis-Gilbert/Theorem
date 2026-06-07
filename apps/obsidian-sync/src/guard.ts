/**
 * Suppresses the plugin's own vault writes so they never echo back through the
 * write-back path. Two layers: a remote-depth counter held for the duration of a
 * pull, and a short per-path TTL that covers the vault `modify`/`create` event that
 * fires a tick after the write returns. The content hash gate in the journal is the
 * primary echo guard; this is the timing backstop.
 */
export class SyncGuard {
  private remoteDepth = 0;
  private suppressed = new Map<string, number>();

  constructor(private ttlMs = 2500) {}

  beginRemote(): void {
    this.remoteDepth += 1;
  }

  endRemote(): void {
    this.remoteDepth = Math.max(0, this.remoteDepth - 1);
  }

  suppress(path: string): void {
    this.suppressed.set(path, Date.now() + this.ttlMs);
  }

  isSuppressed(path: string): boolean {
    if (this.remoteDepth > 0) {
      return true;
    }
    const until = this.suppressed.get(path);
    if (until === undefined) {
      return false;
    }
    if (Date.now() > until) {
      this.suppressed.delete(path);
      return false;
    }
    return true;
  }

  /** Run a vault write with this path suppressed before and after the operation. */
  async write(path: string, op: () => Promise<unknown> | unknown): Promise<void> {
    this.suppress(path);
    try {
      await op();
    } finally {
      // Refresh after the write so the async vault event is still covered.
      this.suppress(path);
    }
  }
}
