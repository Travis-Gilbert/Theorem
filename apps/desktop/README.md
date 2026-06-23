# CommonPlace Desktop Backend Shell

The packaged CommonPlace desktop app is the Tauri backend in `src-tauri/`
wrapping the external Next.js CommonPlace app in `../../../travisgilbert.me`.
`tauri.conf.json` points the main window at that Next dev server in development
and at that app's `out/` static export for packaged builds.

The Vite/React files under `src/` are not the primary product surface anymore.
Keep them only as a typed command-contract/reference harness for Tauri invoke
commands while the CommonPlace panels live in the Next.js app.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
