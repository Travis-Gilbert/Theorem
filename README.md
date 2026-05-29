# Theorem

Theorem planning and implementation spine for Rust-native substrate work.

This repository currently publishes the extracted planning packet from the
Theseus / Index-API workspace. It is intentionally narrow: it carries the Rust
theorem symbolic-engine plan, the Rusty Red Web plan, and the reconciliation
notes that explain the current sequencing.

## Contents

- `docs/plans/rust-theorem-symbolic-engines/` - Rust Theorem: Native Symbolic Engines, including RT-0 through RT-5 status and parity discipline.
- `docs/plans/rusty-red-web/` - Rusty Red Web / RustyWeb implementation plan.
- `docs/plans/commonplace-substrate-reconciliation/` - reconciliation notes that route work between Rust theorem, RustyWeb, and kernel-object lanes.

## Current Direction

Rust theorem hot-path work is implemented through RT-5.2a. Remaining RT-5 ports
stay profile/use-case gated.

RustyWeb is now active work. The first implementation move is to recover the
burst-crawler scaffold as seed code, then build the real product as a RustyRed-
backed graph crawler rather than treating the fetcher scaffold as complete.
