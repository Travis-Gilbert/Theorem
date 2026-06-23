# Launch Ops: index and staging

DRAFT PACK. Cowork drafts and assembles. Travis publishes, posts, and submits.
Nothing here goes live until Travis approves that specific artifact.

Date assembled: 2026-06-21. Source brief: COWORK-HANDOFF-LAUNCH-OPS.

## What is in this folder

| File | Workstream | What it is | Status |
|------|-----------|------------|--------|
| `01-privacy-policy.md` | WS1 | Privacy policy for the connector, the hard gate | Draft ready for review |
| `02-README.md` | WS2 | README as the product page | Draft ready for review |
| `03-launch-posts.md` | WS3 | One narrative, three cuts (HN, Reddit, X) | Draft ready for review |
| `04-connectors-submission-package.md` | WS4 | Connectors Directory submission package | Draft ready, blocked on open values |
| `05-cold-start-amplification.md` | WS5 | Seed list, messages, awesome-list PRs, directories | Draft ready for review |

## Open items, resolved (your four answers)

- Submission path: individual plan. WS4 targets the public MCP directory submission
  form, not the in-product portal (the portal needs a Team or Enterprise org).
- Support contact: GitHub Issues for product, 1travisgilbert@gmail.com for security
  and direct contact.
- Privacy policy hosting: canonical at https://theoremsweb.com/privacy, mirrored on
  GitHub Pages.
- Capture design: written as explicit-tool-call capture. MUST VERIFY against the
  capture code before submitting (see the flag in WS1 and WS4). This is the
  compliance line the directory enforces.

## Open values to fill before WS4 submits

These are placeholders in the drafts. They are values, not decisions. Fill them in
and the submission is a formality. Several are Claude Code's lane (the server).

- `[SERVER_URL]`: the remote HTTPS MCP server on Streamable HTTP. The current
  `commonplace-mcp` is a local stdio binary. The directory requires a remote HTTPS
  Streamable-HTTP endpoint. Standing this up is the release gate, Claude Code's lane.
- `[INSTALL_COMMAND]`: the one-command install pointing at the bundled distribution
  (package or single binary that includes the runtime), not a repo clone.
- `[REPO_URL]`: the public repo URL the README and listing point at.
- `[OAUTH_DETAILS]`: the OAuth user-consent flow, or the custom-connection scheme if
  users supply their own URL or credentials. The directory needs one or the other.
- `[REVIEWER_TEST_URL]` + `[REVIEWER_CREDENTIALS]`: the populated reviewer test
  account and how a reviewer reaches it end to end.
- `[DEMO_GIF]`: the auto-organize clip or the live coworking canvas clip for the
  README and the listing.
- `[BRAND_ASSETS]`: logo, icon, and listing image, sized to the directory spec.

## Submission sequence

The directory review runs on a queue and returns in weeks, not days. WS4 goes early,
not on launch day.

1. Stand up the remote HTTPS MCP server and OAuth (Claude Code). Verify the capture
   design is explicit-tool-call (WS1 flag).
2. Host the privacy policy at https://theoremsweb.com/privacy (WS1). Travis hosts.
3. Fill the open values, submit the WS4 package through the public MCP directory form.
4. Place the README in the repo, set the repo Topics (WS2, WS5). Travis places.
5. Prepare the awesome-list PRs and the seed list (WS5).
6. On launch morning: post the HN cut first, Tuesday through Thursday, morning
   Eastern, then Reddit and X (WS3). Travis posts.

## The guardrail

Cowork does not post to Hacker News, Reddit, or X. Does not submit the directory
form. Does not publish the privacy policy URL. Does not place files in the public
repo. Each of those is Travis's action on Cowork's draft.
