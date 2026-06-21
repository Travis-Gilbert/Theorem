import { useEffect, useRef, useState, useCallback } from "react";
import { useCollaboration } from "@veltdev/tiptap-crdt-react";
import { Editor as TiptapEditor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { askGemma } from "./gemma";

const DOC_ID = (import.meta.env.VITE_DOC_ID as string) || "copresence-demo-doc-1";

// The collaborative editor. `useCollaboration` builds the Velt CRDT (Yjs) store
// for this editorId and returns a Tiptap extension that binds the Y.Doc and
// renders remote cursors. Every peer on the same editorId converges.
export const Editor = () => {
  const elRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<TiptapEditor | null>(null);
  const [thinking, setThinking] = useState(false);
  const [gemmaError, setGemmaError] = useState<string | null>(null);

  const { extension, isLoading, isSynced, status, error } = useCollaboration({
    editorId: DOC_ID,
    initialContent: "<p>Start writing here, then ask Gemma to continue or refine.</p>",
    onError: (e: unknown) => console.error("collaboration error", e),
  });

  useEffect(() => {
    if (!extension || !elRef.current || editorRef.current) return;
    const ed = new TiptapEditor({
      element: elRef.current,
      // Disable Tiptap history: the CRDT owns document state.
      extensions: [StarterKit.configure({ undoRedo: false }), extension],
      content: "",
    });
    editorRef.current = ed;
    return () => {
      ed.destroy();
      editorRef.current = null;
    };
  }, [extension]);

  // Phase 2: browser-driven Gemma. Read the live doc, ask an OpenAI-compatible
  // Gemma endpoint, insert the reply into Tiptap -> Y.Doc -> Velt syncs it to
  // every peer. (Phase 3 makes Gemma its own headless substrate peer.)
  const ask = useCallback(async (instruction: string) => {
    const ed = editorRef.current;
    if (!ed) return;
    setThinking(true);
    setGemmaError(null);
    try {
      const reply = await askGemma(ed.getText(), instruction);
      if (reply) {
        const insert = reply.startsWith(" ") ? reply : ` ${reply}`;
        ed.chain().focus("end").insertContent(insert).run();
      }
    } catch (e) {
      setGemmaError(e instanceof Error ? e.message : String(e));
    } finally {
      setThinking(false);
    }
  }, []);

  if (error) return <div className="err">Collaboration error: {error.message}</div>;
  if (isLoading || !extension) return <div className="status">Connecting to the shared document…</div>;

  return (
    <div className="editor-wrap">
      <div className="status">
        status: {status} · {isSynced ? "synced" : "syncing…"}
      </div>
      <div className="editor" ref={elRef} />
      <div className="gemma-bar">
        <button
          disabled={thinking}
          onClick={() => ask("Continue the document in the same voice. Add one or two sentences.")}
        >
          {thinking ? "Gemma is writing…" : "Ask Gemma to continue"}
        </button>
        <button
          disabled={thinking}
          onClick={() => ask("Improve the last paragraph; return only the replacement sentence(s).")}
        >
          Ask Gemma to refine
        </button>
        {gemmaError && <span className="err">Gemma: {gemmaError}</span>}
      </div>
    </div>
  );
};
