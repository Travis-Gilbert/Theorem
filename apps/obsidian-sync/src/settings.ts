import { App, Notice, PluginSettingTab, Setting } from "obsidian";
import { HarnessClient } from "./harness";
import type TheoremHarnessSyncPlugin from "./main";

export type ConflictMode = "conflict-copy" | "graph-wins" | "local-wins";

/**
 * The commons ("default") tenant is the shared graph; hand-written notes must
 * never land there by accident. Empty is treated as commons too, since the
 * client falls back to "default" when no tenant is set.
 */
export function isCommonsTenant(tenant: string): boolean {
  const slug = tenant.trim().toLowerCase();
  return slug === "" || slug === "default";
}

export interface HarnessSyncSettings {
  /** Harness base URL, e.g. https://rustyredcore-theorem-production.up.railway.app */
  baseUrl: string;
  /** Bearer token scoped to this user's tenant. */
  token: string;
  /** Tenant slug whose partition this vault mirrors. */
  tenant: string;
  /** Vault folder the mirror writes notes into. */
  syncFolder: string;
  /**
   * Folder whose new notes write back to the graph. Empty means "use syncFolder".
   * Combined with the capture flag: a new note writes back if it is in this folder
   * OR carries the capture flag in frontmatter.
   */
  captureFolder: string;
  /** Frontmatter key that opts an individual note into write-back, e.g. `graph: true`. */
  captureFlag: string;
  /** Master switch for Phase 2 write-back. Off by default until the user opts in. */
  enableWriteBack: boolean;
  /**
   * Allow write-back when the tenant is the commons ("default"). Off by default:
   * hand-written notes are blocked from the shared graph unless explicitly opted in.
   */
  allowCommonsWriteback: boolean;
  /** Periodic pull interval in minutes. 0 disables the timer (manual sync only). */
  syncIntervalMinutes: number;
  /** Pull superseded and archived documents too, not just active ones. */
  includeInactive: boolean;
  /** How to resolve a doc that changed on both sides since the last sync. */
  conflictMode: ConflictMode;
  /** Default kind for hand-written new notes with no `kind` in frontmatter. */
  defaultKind: string;
}

export const DEFAULT_SETTINGS: HarnessSyncSettings = {
  baseUrl: "",
  token: "",
  tenant: "default",
  syncFolder: "Theorem",
  captureFolder: "",
  captureFlag: "graph",
  enableWriteBack: false,
  allowCommonsWriteback: false,
  syncIntervalMinutes: 15,
  includeInactive: false,
  conflictMode: "conflict-copy",
  defaultKind: "note",
};

export class HarnessSyncSettingTab extends PluginSettingTab {
  plugin: TheoremHarnessSyncPlugin;

  constructor(app: App, plugin: TheoremHarnessSyncPlugin) {
    super(app, plugin);
    this.plugin = plugin;
  }

  display(): void {
    const { containerEl } = this;
    containerEl.empty();

    containerEl.createEl("h2", { text: "Connection" });

    new Setting(containerEl)
      .setName("Harness base URL")
      .setDesc("Root URL of your harness server (no trailing slash).")
      .addText((text) =>
        text
          .setPlaceholder("https://your-harness.up.railway.app")
          .setValue(this.plugin.settings.baseUrl)
          .onChange(async (value) => {
            this.plugin.settings.baseUrl = value.trim().replace(/\/+$/, "");
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Bearer token")
      .setDesc("Token scoped to your tenant. Stored locally in this vault.")
      .addText((text) => {
        text
          .setPlaceholder("token")
          .setValue(this.plugin.settings.token)
          .onChange(async (value) => {
            this.plugin.settings.token = value.trim();
            await this.plugin.saveSettings();
          });
        text.inputEl.type = "password";
      });

    new Setting(containerEl)
      .setName("Tenant")
      .setDesc("The tenant slug whose memory this vault mirrors.")
      .addText((text) =>
        text
          .setPlaceholder("default")
          .setValue(this.plugin.settings.tenant)
          .onChange(async (value) => {
            this.plugin.settings.tenant = value.trim() || "default";
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Test connection")
      .setDesc("Probe /health and the memory list endpoint, then report the doc count.")
      .addButton((button) =>
        button.setButtonText("Test").onClick(async () => {
          button.setDisabled(true);
          const notice = new Notice("Theorem: testing connection...", 0);
          try {
            const client = new HarnessClient(this.plugin.settings);
            const result = await client.testConnection();
            const sample = result.sampleTitle ? `, e.g. "${result.sampleTitle}"` : "";
            notice.setMessage(
              `Theorem: OK (health ${result.health}). Tenant ` +
                `"${this.plugin.settings.tenant || "default"}" has ` +
                `${result.count} doc(s)${sample}.`
            );
            window.setTimeout(() => notice.hide(), 6000);
          } catch (error) {
            notice.hide();
            new Notice(
              `Theorem: connection failed: ${
                error instanceof Error ? error.message : String(error)
              }`
            );
          } finally {
            button.setDisabled(false);
          }
        })
      );

    containerEl.createEl("h2", { text: "Vault layout" });

    new Setting(containerEl)
      .setName("Sync folder")
      .setDesc("Folder the mirror writes notes into.")
      .addText((text) =>
        text
          .setPlaceholder("Theorem")
          .setValue(this.plugin.settings.syncFolder)
          .onChange(async (value) => {
            this.plugin.settings.syncFolder = normalizeFolder(value);
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Include superseded / archived")
      .setDesc("Also mirror non-active documents (off keeps the vault to current notes).")
      .addToggle((toggle) =>
        toggle
          .setValue(this.plugin.settings.includeInactive)
          .onChange(async (value) => {
            this.plugin.settings.includeInactive = value;
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Auto-sync interval (minutes)")
      .setDesc("How often to pull. 0 disables the timer; sync stays manual.")
      .addText((text) =>
        text
          .setPlaceholder("15")
          .setValue(String(this.plugin.settings.syncIntervalMinutes))
          .onChange(async (value) => {
            const parsed = Number.parseInt(value, 10);
            this.plugin.settings.syncIntervalMinutes = Number.isFinite(parsed)
              ? Math.max(0, parsed)
              : 0;
            await this.plugin.saveSettings();
            this.plugin.restartTimer();
          })
      );

    containerEl.createEl("h2", { text: "Write-back (Phase 2)" });

    if (
      this.plugin.settings.enableWriteBack &&
      isCommonsTenant(this.plugin.settings.tenant) &&
      !this.plugin.settings.allowCommonsWriteback
    ) {
      const warning = containerEl.createEl("p", {
        text:
          'Write-back is on but the tenant is the commons ("default"). ' +
          "Hand-written notes will NOT be pushed. Set a personal tenant above, " +
          'or enable "Allow commons write-back" below to override.',
      });
      warning.style.color = "var(--text-error)";
      warning.style.fontWeight = "600";
    }

    new Setting(containerEl)
      .setName("Enable write-back")
      .setDesc("Push note edits and new linked notes into the graph. Note-linking is graph construction.")
      .addToggle((toggle) =>
        toggle
          .setValue(this.plugin.settings.enableWriteBack)
          .onChange(async (value) => {
            this.plugin.settings.enableWriteBack = value;
            await this.plugin.saveSettings();
            this.display();
          })
      );

    new Setting(containerEl)
      .setName("Allow commons write-back")
      .setDesc(
        'Permit pushing into the commons ("default") tenant. Off by default so ' +
          "hand-written notes never land in the shared graph by accident."
      )
      .addToggle((toggle) =>
        toggle
          .setValue(this.plugin.settings.allowCommonsWriteback)
          .onChange(async (value) => {
            this.plugin.settings.allowCommonsWriteback = value;
            await this.plugin.saveSettings();
            this.display();
          })
      );

    new Setting(containerEl)
      .setName("Capture folder")
      .setDesc("New notes in this folder write back. Empty uses the sync folder.")
      .addText((text) =>
        text
          .setPlaceholder("(sync folder)")
          .setValue(this.plugin.settings.captureFolder)
          .onChange(async (value) => {
            this.plugin.settings.captureFolder = normalizeFolder(value);
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Capture flag")
      .setDesc("Frontmatter key that opts any note into write-back regardless of folder.")
      .addText((text) =>
        text
          .setPlaceholder("graph")
          .setValue(this.plugin.settings.captureFlag)
          .onChange(async (value) => {
            this.plugin.settings.captureFlag = value.trim();
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Default kind")
      .setDesc("Kind given to a hand-written note that sets no `kind` in frontmatter.")
      .addText((text) =>
        text
          .setPlaceholder("note")
          .setValue(this.plugin.settings.defaultKind)
          .onChange(async (value) => {
            this.plugin.settings.defaultKind = value.trim() || "note";
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Conflict resolution")
      .setDesc("What to do when a note and its graph doc both changed since the last sync.")
      .addDropdown((dropdown) =>
        dropdown
          .addOption("conflict-copy", "Write a conflict copy (safe)")
          .addOption("graph-wins", "Graph wins (overwrite local)")
          .addOption("local-wins", "Local wins (skip incoming)")
          .setValue(this.plugin.settings.conflictMode)
          .onChange(async (value) => {
            this.plugin.settings.conflictMode = value as ConflictMode;
            await this.plugin.saveSettings();
          })
      );
  }
}

/** Normalize a folder path: trim, drop leading/trailing slashes, collapse blanks. */
export function normalizeFolder(value: string): string {
  return value.trim().replace(/^\/+/, "").replace(/\/+$/, "");
}
