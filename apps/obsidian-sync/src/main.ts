import { Notice, Plugin, TFile } from "obsidian";
import {
  DEFAULT_SETTINGS,
  HarnessSyncSettingTab,
  HarnessSyncSettings,
} from "./settings";
import { HarnessClient } from "./harness";
import { SyncGuard } from "./guard";
import { Syncer } from "./sync";
import { WriteBack } from "./writeback";
import { emptyJournal, SyncJournal } from "./types";

interface PersistedData {
  settings: HarnessSyncSettings;
  journal: SyncJournal;
}

const WRITE_BACK_DEBOUNCE_MS = 1500;

export default class TheoremHarnessSyncPlugin extends Plugin {
  settings: HarnessSyncSettings = DEFAULT_SETTINGS;
  journal: SyncJournal = emptyJournal();

  private readonly guard = new SyncGuard();
  private client!: HarnessClient;
  private syncer!: Syncer;
  private writeback!: WriteBack;
  private timer: number | null = null;
  private syncing = false;
  private debouncers = new Map<string, number>();

  async onload(): Promise<void> {
    await this.loadState();

    this.client = new HarnessClient(this.settings);
    const persist = () => this.persist();
    this.syncer = new Syncer(
      this.app,
      this.client,
      this.settings,
      this.journal,
      this.guard,
      persist
    );
    this.writeback = new WriteBack(
      this.app,
      this.client,
      this.settings,
      this.journal,
      this.guard,
      persist
    );

    this.addSettingTab(new HarnessSyncSettingTab(this.app, this));

    this.addRibbonIcon("sync", "Theorem harness: sync now", () => {
      void this.syncNow();
    });

    this.addCommand({
      id: "sync-now",
      name: "Sync now (pull from harness)",
      callback: () => void this.syncNow(),
    });

    this.addCommand({
      id: "push-current-note",
      name: "Push current note to the harness",
      checkCallback: (checking) => {
        const file = this.app.workspace.getActiveFile();
        if (!file || file.extension !== "md") {
          return false;
        }
        if (!checking) {
          void this.pushFile(file);
        }
        return true;
      },
    });

    this.addCommand({
      id: "full-resync",
      name: "Full resync (reset watermark and pull all)",
      callback: () => {
        this.journal.watermark = "";
        void this.syncNow();
      },
    });

    this.registerWriteBackEvents();
    this.app.workspace.onLayoutReady(() => this.restartTimer());
  }

  onunload(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer);
    }
    for (const handle of this.debouncers.values()) {
      window.clearTimeout(handle);
    }
    this.debouncers.clear();
  }

  private registerWriteBackEvents(): void {
    const onChange = (file: unknown) => {
      if (file instanceof TFile) {
        this.scheduleWriteBack(file);
      }
    };
    this.registerEvent(this.app.vault.on("modify", onChange));
    this.registerEvent(this.app.vault.on("create", onChange));
    this.registerEvent(this.app.vault.on("rename", onChange));
    this.registerEvent(
      this.app.vault.on("delete", (file) => {
        if (file instanceof TFile) {
          this.handleDelete(file);
        }
      })
    );
  }

  private handleDelete(file: TFile): void {
    if (!this.settings.enableWriteBack || file.extension !== "md") {
      return;
    }
    if (this.guard.isSuppressed(file.path)) {
      return;
    }
    // Cancel any debounced write-back for this path; the file is gone now.
    const pending = this.debouncers.get(file.path);
    if (pending !== undefined) {
      window.clearTimeout(pending);
      this.debouncers.delete(file.path);
    }
    void this.writeback
      .handleDelete(file.path)
      .catch((error) =>
        new Notice(`Theorem delete sync failed: ${errorMessage(error)}`)
      );
  }

  private scheduleWriteBack(file: TFile): void {
    if (!this.settings.enableWriteBack || file.extension !== "md") {
      return;
    }
    if (this.guard.isSuppressed(file.path)) {
      return;
    }
    const existing = this.debouncers.get(file.path);
    if (existing !== undefined) {
      window.clearTimeout(existing);
    }
    const handle = window.setTimeout(() => {
      this.debouncers.delete(file.path);
      const current = this.app.vault.getAbstractFileByPath(file.path);
      if (current instanceof TFile) {
        void this.writeback
          .handleChange(current)
          .catch((error) =>
            new Notice(`Theorem write-back failed: ${errorMessage(error)}`)
          );
      }
    }, WRITE_BACK_DEBOUNCE_MS);
    this.debouncers.set(file.path, handle);
  }

  async syncNow(): Promise<void> {
    if (this.syncing) {
      return;
    }
    if (!this.settings.baseUrl.trim()) {
      new Notice("Theorem: set the harness base URL in settings first.");
      return;
    }
    this.syncing = true;
    const notice = new Notice("Theorem: syncing...", 0);
    try {
      const summary = await this.syncer.pull();
      const filtered = summary.filtered > 0 ? `, ${summary.filtered} filtered` : "";
      notice.setMessage(
        `Theorem: ${summary.created} new, ${summary.updated} updated, ` +
          `${summary.conflicts} conflict(s) (${summary.pulled} pulled${filtered}).`
      );
      window.setTimeout(() => notice.hide(), 4000);
    } catch (error) {
      notice.hide();
      new Notice(`Theorem sync failed: ${errorMessage(error)}`);
    } finally {
      this.syncing = false;
    }
  }

  async pushFile(file: TFile): Promise<void> {
    if (!this.settings.enableWriteBack) {
      new Notice("Theorem: enable write-back in settings first.");
      return;
    }
    try {
      const pushed = await this.writeback.handleChange(file);
      new Notice(pushed ? "Theorem: note pushed to the graph." : "Theorem: nothing to push.");
    } catch (error) {
      new Notice(`Theorem push failed: ${errorMessage(error)}`);
    }
  }

  restartTimer(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
    const minutes = this.settings.syncIntervalMinutes;
    if (minutes > 0) {
      this.timer = window.setInterval(
        () => void this.syncNow(),
        minutes * 60 * 1000
      );
    }
  }

  private async loadState(): Promise<void> {
    const data = (await this.loadData()) as Partial<PersistedData> | null;
    this.settings = Object.assign({}, DEFAULT_SETTINGS, data?.settings ?? {});
    this.journal = data?.journal ?? emptyJournal();
    if (!this.journal.docs) {
      this.journal.docs = {};
    }
    if (typeof this.journal.watermark !== "string") {
      this.journal.watermark = "";
    }
  }

  async persist(): Promise<void> {
    await this.saveData({ settings: this.settings, journal: this.journal });
  }

  async saveSettings(): Promise<void> {
    await this.persist();
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
