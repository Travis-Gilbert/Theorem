# Commonplace Credit Model

This note captures the product direction for credits in Commonplace.

## Product Shape

Use one provider key entry, many model bindings beneath it, and a separate internal machinery registry for OCR, speech, embeddings, ranking, reranking, and search. That keeps Commonplace a room with participants, not a model-picker dashboard.

The user should see a credit cost before the room starts work. The estimate should be honest enough to guide behavior, but it should not expose provider pricing details as the product interface.

## Core Equation

```text
raw_cost_usd =
  sum(input_tokens / 1_000_000 * model_input_rate)
+ sum(output_tokens / 1_000_000 * model_output_rate)
+ tool_costs
+ crawl/runtime overhead

metered_credits = ceil(raw_cost_usd * 1.75 / 0.01)
minimum_credits = 2 + (2 * participant_count) + tool_floor_credits
credits_charged = max(metered_credits, minimum_credits)
```

Draft product policy:

- 1 credit maps to $0.01 retail value.
- New users start with 50 included credits.
- The platform multiplier starts at 1.75 so credits cover provider variance, retries, infrastructure, and payment fees.
- The minimum floor prevents a multi-agent room from feeling free just because one particular prompt was token-light.
- The worst-case estimate should include broaden/escalate stages, not only the warm-start speakers.

## UX Contract

Show the estimate as a range:

```text
6-27 credits
```

The low number is the warm-start path. The high number is the planned worst case if the room needs to broaden or escalate.

Require confirmation for expensive work:

```text
This may use up to 94 credits.
```

The point is not to make users do accounting. The point is to make spending legible before the room starts crawling, reading, OCRing, or calling premium participants.

## Illustrative Runs

These are draft product examples, not billing fixtures:

| Scenario | Shape | Draft result |
| --- | --- | --- |
| Cheap first pass | Qwen Coder Next, small code ask | 4 credits |
| Normal room ask | Qwen Coder Next, Mistral Medium, DeepSeek V4 Pro | 6 credits |
| Coding escalation | Qwen Coder Next, Qwen Coder Large, GLM 5.1, Mistral Medium | 27 credits |
| OCR plus summary | 10 OCR pages plus Mistral Medium | 7 credits |
| Heavy doc synthesis | 30 OCR pages plus Jamba, Mistral, Qwen Coder Large | 94 credits |

The exact numbers will move as provider prices, token estimates, retries, and crawl costs become real telemetry.

## Estimator Contract

The implementation primitive is `CommonplaceCreditEstimator`.

Inputs:

- `CommonplaceRoutePlan`
- `CommonplaceRegistry`
- predicted token budgets
- OCR pages
- speech minutes
- TTS characters
- web fetch count
- substrate search count

Outputs:

- `estimatedCredits`
- `worstCaseCredits`
- `requiresConfirmation`
- raw dollar estimates
- model and tool line items for product explainability

## Important Boundary

The iOS price card is a preview artifact. The billing source of truth should eventually come from the hosted substrate pricing endpoint so provider prices can change without an app release.

The app can still use a local draft price card for first paint, offline preview, and smoke validation. Before charging, the backend should recompute with the current provider price card and return the final charge receipt.
