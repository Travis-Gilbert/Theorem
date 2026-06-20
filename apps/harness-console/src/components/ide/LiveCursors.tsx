"use client";

/**
 * Presence legend for the collaborative IDE. The in-editor remote cursors are
 * rendered by y-codemirror.next from Yjs awareness; this is the participant
 * roster beside the editor (each participant in their own color). It is the
 * seam where Velt's collaboration components drop in over the same Yjs doc.
 */
export interface Participant {
  name: string;
  color: string;
}

export function LiveCursors({ participants }: { participants: Participant[] }) {
  return (
    <div className="flex items-center gap-3">
      {participants.map((p) => (
        <span key={p.name} className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
          <span className="status-dot" style={{ background: p.color }} />
          {p.name}
        </span>
      ))}
    </div>
  );
}
