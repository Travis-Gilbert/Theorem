"use client";

import * as Y from "yjs";
import { Awareness, encodeAwarenessUpdate, applyAwarenessUpdate } from "y-protocols/awareness";

/**
 * The collaborative substrate for the agent IDE. Yjs is the source of truth.
 *
 * To demonstrate a human and a harnessed agent editing the SAME document with
 * separate identities (the spec's acceptance), we model two Yjs clients in one
 * process: a `human` doc bound to CodeMirror, and an `agent` doc that stands in
 * for a server-side Yjs client. Document updates and awareness updates relay
 * both ways with an origin guard, so the human's editor sees the agent's edits
 * and cursor as a genuine remote participant. In production the agent doc is a
 * real server-side client and the relay is the network provider; Velt's cursor
 * components can render over this same awareness without owning the doc.
 */

export interface CollabHandle {
  humanDoc: Y.Doc;
  humanText: Y.Text;
  humanAwareness: Awareness;
  agentText: Y.Text;
  agentAwareness: Awareness;
  destroy: () => void;
}

const RELAY = "relay";

export function createCollab(initial: string): CollabHandle {
  const humanDoc = new Y.Doc();
  const agentDoc = new Y.Doc();
  const humanText = humanDoc.getText("content");
  const agentText = agentDoc.getText("content");

  // Relay document updates both ways (origin guard prevents an echo loop).
  const onHumanUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin !== RELAY) Y.applyUpdate(agentDoc, update, RELAY);
  };
  const onAgentUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin !== RELAY) Y.applyUpdate(humanDoc, update, RELAY);
  };
  humanDoc.on("update", onHumanUpdate);
  agentDoc.on("update", onAgentUpdate);

  // Seed the shared document once (relays to the agent doc).
  if (humanText.length === 0) humanText.insert(0, initial);

  const humanAwareness = new Awareness(humanDoc);
  const agentAwareness = new Awareness(agentDoc);

  // Relay awareness both ways so each side sees the other's cursor.
  const relay = (from: Awareness, to: Awareness) =>
    ({ added, updated, removed }: { added: number[]; updated: number[]; removed: number[] }, origin: unknown) => {
      if (origin === RELAY) return;
      const changed = [...added, ...updated, ...removed];
      const payload = encodeAwarenessUpdate(from, changed);
      applyAwarenessUpdate(to, payload, RELAY);
    };
  const humanAwarenessRelay = relay(humanAwareness, agentAwareness);
  const agentAwarenessRelay = relay(agentAwareness, humanAwareness);
  humanAwareness.on("update", humanAwarenessRelay);
  agentAwareness.on("update", agentAwarenessRelay);

  return {
    humanDoc,
    humanText,
    humanAwareness,
    agentText,
    agentAwareness,
    destroy: () => {
      humanAwareness.destroy();
      agentAwareness.destroy();
      humanDoc.destroy();
      agentDoc.destroy();
    },
  };
}

/**
 * Drive the agent participant: move its cursor and type a snippet into the
 * shared doc, attributed to the agent identity. Returns a stop function.
 * An agent rewriting a function shows up as a second cursor moving through the
 * file, not as a diff dropped in afterward.
 */
export function runAgentParticipant(
  handle: CollabHandle,
  opts: { name: string; color: string; snippet: string },
): () => void {
  handle.agentAwareness.setLocalStateField("user", {
    name: opts.name,
    color: opts.color,
    colorLight: `${opts.color}33`,
  });

  let i = 0;
  let cancelled = false;
  const tick = () => {
    if (cancelled) return;
    const len = handle.agentText.length;
    const at = Math.min(len, Math.max(0, Math.floor(len * 0.4)));
    // Move the agent cursor using yCollab's relative-position format so the
    // human editor's remote-cursor layer renders it correctly.
    const rel = Y.relativePositionToJSON(Y.createRelativePositionFromTypeIndex(handle.agentText, at));
    handle.agentAwareness.setLocalStateField("cursor", { anchor: rel, head: rel });
    if (i < opts.snippet.length) {
      const ch = opts.snippet.slice(i, i + 2);
      handle.agentText.insert(Math.min(handle.agentText.length, at), ch);
      i += 2;
    } else {
      i = 0;
    }
  };
  const id = setInterval(tick, 240);
  return () => {
    cancelled = true;
    clearInterval(id);
  };
}
