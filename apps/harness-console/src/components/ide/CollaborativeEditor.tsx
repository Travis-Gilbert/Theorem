"use client";

import * as React from "react";
import CodeMirror, { EditorView } from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { javascript } from "@codemirror/lang-javascript";
import { yCollab } from "y-codemirror.next";
import type { Extension } from "@codemirror/state";
import { createCollab, runAgentParticipant, type CollabHandle } from "./collab";
import { LiveCursors, type Participant } from "./LiveCursors";
import type { EditorLanguage } from "@/components/editor/MarkdownEditor";

/**
 * The collaborative agent IDE. The document is a Yjs doc and Yjs is the source
 * of truth. The human edits through CodeMirror 6 bound with y-codemirror.next;
 * the agent is a participant with its own awareness identity, so it appears with
 * its own cursor and selection as it rewrites the file. Presence and live
 * cursors come from the Yjs awareness (the Velt cursor components can render
 * over this same doc without owning it).
 */

const HUMAN: Participant = { name: "You", color: "#1a1a1d" };
const AGENT: Participant = { name: "Theorem agent", color: "#8a2e29" };

const cmTheme = EditorView.theme({
  "&": { backgroundColor: "transparent", color: "var(--ink)", fontSize: "13.5px" },
  ".cm-content": { fontFamily: "var(--font-plex-mono), monospace", padding: "12px 0" },
  ".cm-gutters": { backgroundColor: "transparent", color: "var(--faint)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--surface)" },
  "&.cm-focused": { outline: "none" },
  ".cm-ySelectionInfo": { fontFamily: "var(--font-plex-mono), monospace", fontSize: "10px" },
});

function lang(language: EditorLanguage): Extension[] {
  if (language === "markdown") return [markdown()];
  if (language === "javascript" || language === "typescript") return [javascript({ typescript: language === "typescript" })];
  return [];
}

export function CollaborativeEditor({
  initialDoc,
  language = "typescript",
  agentSnippet = "\n// agent: tightened the guard and added a receipt\n",
  minHeight = "320px",
}: {
  initialDoc: string;
  language?: EditorLanguage;
  agentSnippet?: string;
  minHeight?: string;
}) {
  const [mounted, setMounted] = React.useState(false);
  const [agentLive, setAgentLive] = React.useState(true);
  const handleRef = React.useRef<CollabHandle | null>(null);
  const stopRef = React.useRef<(() => void) | null>(null);
  const [extensions, setExtensions] = React.useState<Extension[] | null>(null);

  React.useEffect(() => {
    setMounted(true);
    const handle = createCollab(initialDoc);
    handleRef.current = handle;
    handle.humanAwareness.setLocalStateField("user", {
      name: HUMAN.name,
      color: HUMAN.color,
      colorLight: `${HUMAN.color}22`,
    });
    setExtensions([...lang(language), EditorView.lineWrapping, yCollab(handle.humanText, handle.humanAwareness)]);

    return () => {
      stopRef.current?.();
      handle.destroy();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  React.useEffect(() => {
    const handle = handleRef.current;
    if (!handle) return;
    if (agentLive) {
      stopRef.current = runAgentParticipant(handle, { name: AGENT.name, color: AGENT.color, snippet: agentSnippet });
    } else {
      stopRef.current?.();
      stopRef.current = null;
    }
    return () => stopRef.current?.();
  }, [agentLive, agentSnippet]);

  if (!mounted || !extensions) {
    return <div className="animate-[pulse_1.5s_ease-in-out_infinite] rounded-md border border-line bg-surface" style={{ minHeight }} />;
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <LiveCursors participants={agentLive ? [HUMAN, AGENT] : [HUMAN]} />
        <button
          onClick={() => setAgentLive((v) => !v)}
          className="rounded-md border border-line px-2 py-1 font-mono text-[11px] text-muted-foreground hover:bg-surface-2 hover:text-ink"
        >
          {agentLive ? "pause agent" : "resume agent"}
        </button>
      </div>
      <div className="overflow-hidden rounded-md border border-line bg-bg">
        <CodeMirror
          value={initialDoc}
          theme={cmTheme}
          extensions={extensions}
          basicSetup={{ lineNumbers: true, foldGutter: false }}
          style={{ minHeight }}
        />
      </div>
      <p className="text-[11px] text-muted-foreground">
        Document state lives in Yjs and survives without the presence layer. Edits from the agent arrive as live cursor
        movement attributed to it, not a diff dropped in afterward.
      </p>
    </div>
  );
}
