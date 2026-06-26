"use client";

import { EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { javascript } from "@codemirror/lang-javascript";
import { markdown } from "@codemirror/lang-markdown";

export type CommonPlaceCodeLanguage = "javascript" | "typescript" | "markdown" | "rust" | "text";

export const commonplaceCodeMirrorTheme = EditorView.theme({
  "&": {
    backgroundColor: "transparent",
    color: "var(--cp-text)",
    fontSize: "12.5px",
  },
  ".cm-scroller": {
    fontFamily: "var(--cp-font-mono)",
    lineHeight: "1.58",
  },
  ".cm-content": {
    padding: "var(--cp-space-3)",
    caretColor: "var(--cp-red)",
  },
  ".cm-gutters": {
    backgroundColor: "rgba(58, 50, 42, 0.035)",
    color: "var(--cp-muted)",
    borderRight: "1px solid var(--cp-line)",
  },
  ".cm-activeLine": {
    backgroundColor: "rgba(127, 39, 31, 0.055)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "rgba(127, 39, 31, 0.075)",
  },
  ".cm-selectionBackground, ::selection": {
    backgroundColor: "rgba(127, 39, 31, 0.16) !important",
  },
  ".cm-cursor": {
    borderLeftColor: "var(--cp-red)",
  },
  ".cm-line": {
    paddingLeft: "var(--cp-space-2)",
    paddingRight: "var(--cp-space-2)",
  },
  ".cm-diagnostic": {
    borderLeftColor: "var(--cp-red)",
  },
  ".cm-changedLine": {
    backgroundColor: "rgba(55, 102, 114, 0.08)",
  },
  ".cm-deletedChunk": {
    backgroundColor: "rgba(127, 39, 31, 0.07)",
  },
  "&.cm-focused": {
    outline: "none",
  },
});

export const readOnlyExtensions: readonly Extension[] = [
  EditorView.editable.of(false),
  EditorState.readOnly.of(true),
];

export function commonplaceCodeExtensions(language: CommonPlaceCodeLanguage = "typescript"): Extension[] {
  const languageExtensions =
    language === "markdown"
      ? [markdown()]
      : language === "javascript" || language === "typescript"
        ? [javascript({ jsx: true, typescript: language === "typescript" })]
        : [];

  return [...languageExtensions, EditorView.lineWrapping];
}
