// Feature flags for surfaces that ride the native Servo browser-use lane
// (docs/plans/servo-browser-use-agent/, jobs 007-009). The agent surface
// (co-browse, the pre-action veto) is built now so the affordance exists the
// day that lane lands, but stays gated until it does. Flip to true to exercise
// it from fixtures.
export const AGENT_SURFACE_ENABLED = false;
