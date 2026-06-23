# Bundled fonts

The instrument-register stack (SPEC-THEOREM-IOS-V1-ADDENDUM-DESIGN.md) ships in
the binary. All faces are under the SIL Open Font License (OFL), so each is
committed here with its license file:

| File | Family | Role | License |
|------|--------|------|---------|
| `Karrik-Regular.ttf` | Karrik | display (hero, section headers, surface titles) | `OFL-Karrik.txt` |
| `IBMPlexSans.ttf` | IBM Plex Sans (variable `wght`/`wdth`) | body + instrument labels | `OFL-IBMPlexSans.txt` |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono | data readouts (numbers, IDs, edge values) | `OFL-JetBrainsMono.txt` |
| `TerminalGrotesque-Regular.ttf` | Terminal Grotesque | code/flavor labels, scramble-text | `OFL-TerminalGrotesque.txt` |
| `jgs9.ttf` | jgs | patent-frame ornament (optional texture) | `OFL-jgs.txt` |

Karrik replaces Archivo Black (and, before it, Berthold Akzidenz-Grotesk): it
carries the display + header sizes without the heaviness the instrument register
rejects. The family/PostScript names above match the `TheoremFonts` tokens
exactly, which is what `.custom(name:)` resolves against.

The faces are registered at launch via `TheoremFonts.registerBundledFonts()`
(SwiftPM resources are not auto-registered the way an app target's `UIAppFonts`
entries are). If a face fails to load, the type tokens fall back to the system
face — honest degradation, never a crash.

## Adding a font

Commercial fonts without an embedding license must NOT be added. Only OFL (or
otherwise embed-licensed) fonts, and always commit the license file alongside.
