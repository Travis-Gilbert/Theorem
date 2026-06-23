import type { RecallHit } from "../state/types";

/** Known-context strip (D4): prior harness memories for the active domain. Text only. */
export function KnownContextStrip({ hits }: { hits: RecallHit[] }) {
  if (!hits || hits.length === 0) return null;
  return (
    <div className="known">
      <div className="known__label">Known context</div>
      {hits.slice(0, 4).map((h) => (
        <div className="known__item" key={h.id} title={h.snippet}>
          {h.title}
          {h.snippet ? ` -- ${h.snippet}` : ""}
        </div>
      ))}
    </div>
  );
}
