import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import { PROVIDERS, type HarnessTarget, type ReceiverSettings } from "../state/types";
import { useApp } from "../state/store";

function targetLabel(target: HarnessTarget): string {
  return target === "local" ? "Local node" : "Hosted";
}

export function Settings() {
  const { state, actions } = useApp();
  const { settings } = state;
  const [nodeStatus, setNodeStatus] = useState<cmd.LocalNodeStatus | null>(null);
  const [receiverStatus, setReceiverStatus] = useState<cmd.ReceiverStatus | null>(null);
  const [connectorProof, setConnectorProof] =
    useState<cmd.ConnectorProofResult | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function refresh() {
      const [node, receiver, backendReceiver] = await Promise.all([
        cmd.localNodeStatus(),
        cmd.receiverStatus(),
        cmd.receiverSettingsGet(),
      ]);
      if (cancelled) return;
      setNodeStatus(node);
      setReceiverStatus(receiver);
      if (backendReceiver) actions.setReceiver(backendReceiver);
    }

    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [actions]);

  const persistHarness = (patch: Partial<typeof settings.harness>) => {
    const next = { ...settings.harness, ...patch };
    actions.setHarness(next);
    void cmd.harnessSettingsSet(next);
  };

  const persistReceiver = (patch: Partial<ReceiverSettings>) => {
    const next = { ...settings.receiver, ...patch };
    actions.setReceiver(next);
    void cmd.receiverSettingsSet(next).then(() => cmd.receiverStatus().then(setReceiverStatus));
  };

  const persistSync = (patch: Partial<typeof settings.sync>) => {
    actions.setSync(patch);
  };

  const persistBackgroundFetch = (patch: Partial<typeof settings.backgroundFetch>) => {
    actions.setBackgroundFetch(patch);
  };

  const persistOllama = (patch: Partial<typeof settings.ollama>) => {
    actions.setOllama(patch);
  };

  const theoremWorktree = settings.receiver.worktrees["Travis-Gilbert/theorem"] ?? "";

  return (
    <div className="settings" role="dialog" aria-modal="true">
      <section className="settings__panel">
        <header className="settings__head">
          <h2>Settings</h2>
          <button className="iconbtn" type="button" onClick={() => actions.openSettings(false)}>
            Close
          </button>
        </header>

        <section className="settings__group">
          <h3>Harness</h3>
          <div className="settings__status">
            <span className={nodeStatus?.nodeUp ? "badge badge--on" : "badge"}>
              {nodeStatus?.nodeUp ? "Node up" : "Node off"}
            </span>
            <span className="settings__muted">{targetLabel(settings.harness.activeTarget)}</span>
          </div>
          <label className="field">
            <span>Memory target</span>
            <select
              value={settings.harness.activeTarget}
              onChange={(event) =>
                persistHarness({ activeTarget: event.currentTarget.value as HarnessTarget })
              }
            >
              <option value="hosted">Hosted</option>
              <option value="local">Local node</option>
            </select>
          </label>
          <label className="field">
            <span>Hosted MCP</span>
            <input
              value={settings.harness.endpoint}
              onChange={(event) => persistHarness({ endpoint: event.currentTarget.value })}
            />
          </label>
          <label className="field">
            <span>Local MCP</span>
            <input
              value={settings.harness.localEndpoint}
              onChange={(event) => persistHarness({ localEndpoint: event.currentTarget.value })}
            />
          </label>
          <label className="field">
            <span>Tenant</span>
            <input
              value={settings.harness.tenant}
              onChange={(event) => persistHarness({ tenant: event.currentTarget.value })}
            />
          </label>
          <div className="settings__kv">
            <span>Store</span>
            <code>{nodeStatus?.storePath ?? "..."}</code>
            <span>Tools parity</span>
            <code>{nodeStatus?.toolsMatchHosted ? "matching" : "not verified"}</code>
          </div>
        </section>

        <section className="settings__group">
          <h3>Receiver</h3>
          <div className="settings__status">
            <span className={settings.receiver.enabled ? "badge badge--on" : "badge"}>
              {receiverStatus?.state ?? "off"}
            </span>
            <span className="settings__muted">
              {(receiverStatus?.lanes ?? []).join(", ") || "No lanes detected"}
            </span>
          </div>
          <label className="settings__check">
            <input
              type="checkbox"
              checked={settings.receiver.enabled}
              onChange={(event) => persistReceiver({ enabled: event.currentTarget.checked })}
            />
            <span>Receiver on</span>
          </label>
          <label className="field">
            <span>Claim interval</span>
            <input
              type="number"
              min={5}
              value={settings.receiver.claimIntervalSecs}
              onChange={(event) =>
                persistReceiver({ claimIntervalSecs: Number(event.currentTarget.value) || 20 })
              }
            />
          </label>
          <label className="field">
            <span>Theorem worktree</span>
            <input
              value={theoremWorktree}
              onChange={(event) =>
                persistReceiver({
                  worktrees: {
                    ...settings.receiver.worktrees,
                    "Travis-Gilbert/theorem": event.currentTarget.value,
                  },
                })
              }
            />
          </label>
          <div className="settings__kv">
            <span>Last claim</span>
            <code>{receiverStatus?.lastClaimTime ?? "none"}</code>
            <span>Last result</span>
            <code>{receiverStatus?.lastJobResult ?? "none"}</code>
          </div>
        </section>

        <section className="settings__group">
          <h3>Sync</h3>
          <label className="settings__check">
            <input
              type="checkbox"
              checked={settings.sync.enabled}
              onChange={(event) => persistSync({ enabled: event.currentTarget.checked })}
            />
            <span>Local-hosted sync</span>
          </label>
          <label className="field">
            <span>Interval</span>
            <input
              type="number"
              min={60}
              value={settings.sync.intervalSecs}
              onChange={(event) =>
                persistSync({ intervalSecs: Number(event.currentTarget.value) || 300 })
              }
            />
          </label>
          <button type="button" onClick={() => void actions.runSync()}>
            Run sync
          </button>
          <div className="settings__kv">
            <span>Latest</span>
            <code>{state.syncReceipts[0]?.message ?? "off"}</code>
          </div>
        </section>

        <section className="settings__group">
          <h3>Models</h3>
          <label className="field">
            <span>Default model</span>
            <select
              value={settings.defaultModel}
              onChange={(event) =>
                actions.setDefaultModel(event.currentTarget.value as typeof settings.defaultModel)
              }
            >
              {PROVIDERS.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.label}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Ollama endpoint</span>
            <input
              value={settings.ollama.endpoint}
              onChange={(event) => persistOllama({ endpoint: event.currentTarget.value })}
            />
          </label>
          <label className="field">
            <span>Ollama model</span>
            <input
              value={settings.ollama.model}
              onChange={(event) => persistOllama({ model: event.currentTarget.value })}
            />
          </label>
          <div className="settings__kv">
            <span>Session cost</span>
            <code>${state.costSummary.estimatedUsd.toFixed(4)}</code>
            <span>Tokens</span>
            <code>
              {state.costSummary.tokensIn}/{state.costSummary.tokensOut}
            </code>
          </div>
        </section>

        <section className="settings__group">
          <h3>Background</h3>
          <label className="settings__check">
            <input
              type="checkbox"
              checked={settings.backgroundFetch.enabled}
              onChange={(event) =>
                persistBackgroundFetch({ enabled: event.currentTarget.checked })
              }
            />
            <span>Fetch warmups</span>
          </label>
          <label className="field">
            <span>Interval</span>
            <input
              type="number"
              min={120}
              value={settings.backgroundFetch.intervalSecs}
              onChange={(event) =>
                persistBackgroundFetch({
                  intervalSecs: Number(event.currentTarget.value) || 900,
                })
              }
            />
          </label>
          <button
            type="button"
            onClick={() => void cmd.connectorProofRun().then(setConnectorProof)}
          >
            Connector proof
          </button>
          <div className="settings__kv">
            <span>Proof</span>
            <code>{connectorProof?.message ?? "not run"}</code>
            <span>Agent ingests</span>
            <code>{state.agentIngestionReceipts.length}</code>
          </div>
        </section>
      </section>
    </div>
  );
}
