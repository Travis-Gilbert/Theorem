# Bundled fonts

Two faces ship in the binary, both under the SIL Open Font License (OFL), so
they are committed here with their license files:

| File | Family | Role | License |
|------|--------|------|---------|
| `ArchivoBlack-Regular.ttf` | Archivo Black | display (headers, Dynamic Island, surface titles) | `OFL-ArchivoBlack.txt` |
| `IBMPlexSans.ttf` | IBM Plex Sans (variable `wght`/`wdth`) | body | `OFL-IBMPlexSans.txt` |

Archivo Black is the free stand-in for Berthold Akzidenz-Grotesk, which has no
embedding license available and is therefore NOT shipped (the spec forbids
shipping it without the license). `mono` uses the system monospaced face.

The faces are registered at launch via `TheoremFonts.registerBundledFonts()`
(SwiftPM resources are not auto-registered the way an app target's `UIAppFonts`
entries are). If a face fails to load, the type tokens fall back to the system
face — honest degradation, never a crash.

## Adding a font

Commercial fonts without an embedding license must NOT be added. Only OFL (or
otherwise embed-licensed) fonts, and always commit the license file alongside.
