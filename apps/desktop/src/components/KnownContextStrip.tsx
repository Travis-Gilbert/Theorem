import { useEffect, useState } from "react";
import type { RecallHit } from "../state/types";
import { CloseIcon } from "./icons";

/**
 * Known-context strip (D4.1): prior harness memories for the active domain.
 * Text only, no graph. Grounded vs unverified hits are distinguished
 * STRUCTURALLY (border style + color), not by opacity, so the tier survives
 * dark mode and color-vision differences. Dismissible per turn.
 */
export function KnownContextStrip({ hits }: { hits: RecallHit[] }) {
  const [dismissed, setDismissed] = useState(false);

  // A new set of hits (new domain/turn) brings the strip back.
  const key = hits.map((h) => h.id).join(",");
  useEffect(() => {
    setDismissed(false);
  }, [key]);

  if (!hits || hits.length === 0 || dismissed) return null;

  return (
    <div className="known">
      <div className="known__head">
        <span className="known__label">Known context</span>
        <button
          className="iconbtn known__dismiss"
          onClick={() => setDismissed(true)}
          title="Dismiss for this turn"
          aria-label="Dismiss known context"
        >
          <CloseIcon size={13} />
        </button>
      </div>
      <div className="known__list">
        {hits.slice(0, 4).map((h) => {
          const unverified = h.tier === "unverified";
          return (
            <div
              className={
                "known__item " +
                (unverified ? "known__item--unverified" : "known__item--grounded")
              }
              key={h.id}
              title={h.snippet}
            >
              {unverified && <span className="known__tier">unverified</span>}
              <span className="known__text">
                {h.title}
                {h.snippet ? ` -- ${h.snippet}` : ""}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
