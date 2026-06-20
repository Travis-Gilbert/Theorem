"use client";

import * as React from "react";
import CodeMirror, { EditorView } from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { javascript } from "@codemirror/lang-javascript";
import type { Extension } from "@codemirror/state";
import { useIsClient } from "@/lib/hooks/useIsClient";

/**
 * CodeMirror 6 editor, shared by Memory (prose markdown) and Skills (SKILL.md +
 * code files). One component, two configurations. It sits in the calm content
 * zone with muted warm syntax rather than a saturated dark theme. Renders only
 * after mount so it never touches the DOM during SSR.
 */

export type EditorLanguage = "markdown" | "javascript" | "typescript" | "rust" | "text";

// Calm, light theme keyed to the console tokens. Muted syntax, hairline gutter.
const calmTheme = EditorView.theme({
  "&": { backgroundColor: "transparent", color: "var(--ink)", fontSize: "13.5px" },
  ".cm-content": { fontFamily: "var(--font-plex-mono), monospace", padding: "12px 0" },
  ".cm-gutters": { backgroundColor: "transparent", color: "var(--faint)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--surface)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "var(--ox-tint) !important" },
  ".cm-cursor": { borderLeftColor: "var(--ox)" },
  "&.cm-focused": { outline: "none" },
  ".cm-line": { lineHeight: "1.6" },
});

function langExtension(language: EditorLanguage): Extension[] {
  switch (language) {
    case "markdown":
      return [markdown()];
    case "javascript":
    case "typescript":
      return [javascript({ typescript: language === "typescript" })];
    default:
      return [];
  }
}

export function MarkdownEditor({
  value,
  onChange,
  language = "markdown",
  readOnly = false,
  minHeight = "240px",
  extraExtensions = [],
}: {
  value: string;
  onChange?: (v: string) => void;
  language?: EditorLanguage;
  readOnly?: boolean;
  minHeight?: string;
  extraExtensions?: Extension[];
}) {
  const isClient = useIsClient();

  if (!isClient) {
    return (
      <div
        className="animate-[pulse_1.5s_ease-in-out_infinite] rounded-md border border-line bg-surface"
        style={{ minHeight }}
      />
    );
  }

  return (
    <div className="overflow-hidden rounded-md border border-line bg-bg">
      <CodeMirror
        value={value}
        onChange={onChange}
        readOnly={readOnly}
        theme={calmTheme}
        extensions={[...langExtension(language), EditorView.lineWrapping, ...extraExtensions]}
        basicSetup={{ lineNumbers: language !== "markdown", foldGutter: false, highlightActiveLine: !readOnly }}
        style={{ minHeight }}
      />
    </div>
  );
}
