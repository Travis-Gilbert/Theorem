import { useEffect, useRef, useState } from "react";

// The pre-action preview: the browse_with_me veto surface (D4.3). Built now from
// an ActionCandidate fixture, behind AGENT_SURFACE_ENABLED, so the human's
// veto/approve affordance exists before the live agent session (job-007) lands.

export type ActionRisk =
  | "read_only"
  | "external_web"
  | "hot_graph_write"
  | "canonical_write"
  | "remember"
  | "state_changing";

export interface ActionCandidate {
  id: string;
  /** Human-readable action, e.g. 'Click "Add to cart"'. */
  actionLabel: string;
  /** The element the action targets, e.g. 'button.add-to-cart'. */
  targetElement: string;
  risk: ActionRisk;
}

const RISK_LABEL: Record<ActionRisk, string> = {
  read_only: "read only",
  external_web: "external web",
  hot_graph_write: "graph write",
  canonical_write: "canonical write",
  remember: "remember",
  state_changing: "state changing",
};

// Risk -> chip treatment. Fills and borders only, never accent-colored small
// text (the D2 usage rule). read_only is neutral; external_web is agent/brass;
// the graph-writing risks carry a memory/green border; state_changing is danger.
function riskClass(risk: ActionRisk): string {
  switch (risk) {
    case "read_only":
      return "risk-chip risk-chip--read";
    case "external_web":
      return "risk-chip risk-chip--agent";
    case "hot_graph_write":
    case "canonical_write":
    case "remember":
      return "risk-chip risk-chip--memory";
    case "state_changing":
      return "risk-chip risk-chip--danger";
  }
}

const FIXTURE: ActionCandidate = {
  id: "fixture-add-to-cart",
  actionLabel: 'Click "Add to cart"',
  targetElement: "button.add-to-cart",
  risk: "state_changing",
};

interface Props {
  candidate?: ActionCandidate;
  onApprove?: (candidate: ActionCandidate) => void;
  onVeto?: (candidate: ActionCandidate) => void;
}

export function PreActionPreview({ candidate = FIXTURE, onApprove, onVeto }: Props) {
  const [open, setOpen] = useState(true);
  const approveRef = useRef<HTMLButtonElement>(null);

  // Focus lands on Approve when the preview appears.
  useEffect(() => {
    if (open) approveRef.current?.focus();
  }, [open, candidate.id]);

  if (!open) return null;

  const approve = () => {
    onApprove?.(candidate);
    setOpen(false);
  };
  const veto = () => {
    onVeto?.(candidate);
    setOpen(false);
  };

  return (
    <div
      className="preaction"
      role="alertdialog"
      aria-label="Confirm agent action"
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          approve();
        } else if (e.key === "Escape") {
          e.preventDefault();
          veto();
        }
      }}
    >
      <div className="preaction__head">
        <span className="preaction__label">{candidate.actionLabel}</span>
        <span className={riskClass(candidate.risk)}>{RISK_LABEL[candidate.risk]}</span>
      </div>
      <div className="preaction__target">
        on <code>{candidate.targetElement}</code>
      </div>
      <div className="preaction__actions">
        <button ref={approveRef} className="btn btn--primary" onClick={approve}>
          Approve
        </button>
        <button className="btn" onClick={veto}>
          Take the wheel
        </button>
      </div>
      <div className="preaction__hint">Enter approves, Esc takes the wheel.</div>
    </div>
  );
}
