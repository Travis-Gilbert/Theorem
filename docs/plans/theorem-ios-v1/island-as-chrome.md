# Island as Chrome: the Theorem iOS interaction model

Status: settled design intent (2026-06-01). Artifact for Claude Code + Codex to
build against so neither reinvents a config UI the user is never meant to see.
Code landed in commit `5d23c28` (island + registry integration).

## Principle

The Dynamic Island (the "search box") is the single chrome surface. The graph is
the canvas. **All chrome exists through the box changing shape and context**, not
as separate slabs below the graph. The old ROOM slab and header slab are gone.

## The box: one surface, several shapes

| Mode | What it is |
|------|-----------|
| `idle` | the collapsed pill. Shows the active room's ask + a search affordance. |
| `room` | the active room's conversation: ask, participant presence, recent contributions, a read-only credit estimate. |
| `detail` (dossier) | a tapped node's identity + choices (Summary / SceneOS / Search / Link). |
| `search` | text input + the algorithm/projection switcher (honest-shape gated). |
| `ask` | text input (talk), with a target: the room, or a selected model. |

Entry: tap the pill main area to open the room; tap a node to open its dossier;
tap the search half to open search. Same box, different shape.

## What the box is FOR (the load-bearing rule): read + talk

For the **user**, the box does exactly two things:

- **READ** summaries: a node, the room, a model, the credit estimate.
- **TALK**: search, ask, address a specific model.

It never hosts configuration.

## The read / write split

- The user **reads** almost everything (summaries) and **talks** (input).
- The user **writes** exactly one thing: their **endpoints + API keys**.
- Everything else is authored by **us**, in code/backend, never an app surface:
  model bindings, machinery (OCR / STT / TTS / embedding / ranking), the room
  charter, routing, and the credit math.

This is why "select a model to configure it" mostly does not exist for the user:
configuring a model is not a user action.

## Registry: read-only in the box

The registry (charter / team / machinery) is a developer concept. To the user it
is **read-only**: present in the box as a summary (the team chips, the credit
estimate, "this room can do X for Y credits"). It is never edited in the box, or
anywhere in the app. Its authorship lives with us, in code.

## Models (the room's team)

- Each model/participant is summarized in the room as a presence chip.
- **Light tap a model -> the input box, targeted at that model.** This is the
  `addressBroughtAgent` path, gated by `CommonplaceParticipant.isDirectlyAddressable`.
  It is "talk," and it is box-native.
- There is **no** user "configure this model" affordance, because config is ours.
- The input box gains a **target**: "Ask the room" (default) or "Address CX"
  (a selected model). Same input, a context flag.

## The one user-writable surface: keys + endpoints

A small, bounded BYO-credentials screen (NOT a registry editor), living **outside
the box**:

- **API keys -> Keychain.** User secrets; never UserDefaults / plaintext.
- **Endpoint URLs -> config.** The existing UserDefaults `searchBaseURL` path is
  fine for non-secret URLs.

## Credit / CONFIRM stays user-facing

The credit estimate + CONFIRM gate is a **read + a consent**, not config: the user
sees "this will cost ~N credits" and confirms before spend. Box-native (read a
summary, take one consequential action). Already on screen in the room
(`IslandRoomView` credit strip, `CommonplaceCreditEstimator`).

## Open questions

1. **Multiple rooms.** "The registry in the box" must resolve to the *active*
   room's registry. Context binding to settle when more than one room exists.
2. **Room/registry are guests in the box, not citizens.** The box's hard
   commitment is search + chat + dossier. Room is hosted there now because it
   works and Travis wants to use it, but room/registry have a different
   interaction model than search/chat and may want their own surface as they
   grow. Treat the in-box room as provisional.

## Ownership

- **Registry model + interaction**: Codex (`CommonplaceRegistry`, `CommonplaceRouter`,
  `CommonplaceCreditEstimator`, the `CommonplaceRoom` registry fields).
- **Island chrome** (the box, its modes, the dossier, the room render): Claude Code
  (`DynamicIslandView`, `IslandRoomView`, `NodeDossierView`, `TheoremRootView`).

The two lanes converged at `5d23c28`. This note is the shared contract.
