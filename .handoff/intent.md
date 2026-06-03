# Handoff pipeline test

This is a deliberately trivial task to verify the claude.ai handoff pipeline end to end (push to .handoff -> relay -> repository_dispatch -> claude-code-action) on a cheap, fast run before a real one.

Create a file named `HELLO.md` at the repository root containing exactly one line:

Handoff pipeline verified from claude.ai.

Then open a pull request against `main` with just that change. Do NOT merge it. Keep it minimal: no other files, no analysis, no extra commits, no dependency installs.
