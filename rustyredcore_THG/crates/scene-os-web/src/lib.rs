//! SceneOS renderer serving — turn a scene package into the browser's scene PAGE.
//!
//! This is Lane B of the SceneOS -> Theorem port: the renderer half. Lane A
//! (`scene-os-core`) is the director that produces a [`ScenePackageV2`]; this
//! crate takes that package and serves the page that DRAWS it. The browser's
//! `load_web_resource` hook (Lane C) intercepts a scene URL, calls Lane A to
//! produce the package, and serves the HTML this module returns — exactly as
//! `rustyred-web` serves its SERP graph page.
//!
//! The page is a single self-contained HTML document (`web/scene-host.html`,
//! embedded via `include_str!`) with the renderer bundle (`web/dist/
//! scene-os.bundle.js`, an esbuild IIFE with d3 inlined) injected in place of a
//! placeholder. No bundler at serve time, no npm, no CDN: Servo serves one
//! asset. The only dynamic part is the scene package, injected in place of a
//! `null` marker.
//!
//! Security: the page renders atom labels / kinds that may come from CRAWLED
//! pages or agent output, which are untrusted. Two defenses, both required and
//! mirroring `rustyred-web::serp`:
//!   1. `scene-host.html` + the bundle set every piece of DOM text via
//!      `textContent` / `createElement`, never `innerHTML`.
//!   2. [`scene_payload_json`] escapes `<`, `>`, `&` to their `\uXXXX` forms so
//!      a label containing `</script>` cannot break out of the `<script>` block
//!      the payload is injected into.

use scene_os_core::ScenePackageV2;

/// The self-contained scene host page. The `null` payload marker and the bundle
/// placeholder are replaced at render time. Served verbatim (honest empty
/// state) when no package is supplied.
const SCENE_TEMPLATE: &str = include_str!("../web/scene-host.html");

/// The renderer bundle (esbuild IIFE, d3 inlined), built from `web/src` via
/// `web/build.mjs`. Committed so the crate is self-contained, mirroring
/// `rustyred-web`'s vendored `d3.min.js`.
const SCENE_BUNDLE: &str = include_str!("../web/dist/scene-os.bundle.js");

/// The line in the template carrying the payload placeholder.
const PAYLOAD_MARKER: &str = "window.__SCENE_PACKAGE__ = null; // __SCENE_PACKAGE__";

/// The placeholder where the renderer bundle is inlined.
const BUNDLE_MARKER: &str = "/*__SCENE_OS_BUNDLE__*/";

/// Serialize a JSON string to a `<script>`-safe literal.
///
/// `serde_json` does not escape `<`/`>`/`&` by default, so an atom label
/// containing `</script>` would close the script tag the payload lives in. We
/// escape those three to their JSON `\uXXXX` forms — still valid JSON (so it
/// re-parses), still renders as the original character in JS, but inert as HTML.
///
/// Input must already be valid JSON (e.g. from `serde_json::to_string`).
pub fn scene_payload_json(package_json: &str) -> String {
    package_json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Render the browser's scene page for an already-serialized scene package
/// (the JSON Lane A's director emits, or the Python director transitionally).
///
/// This is the engine-agnostic entry: it draws whatever atoms + projection the
/// JSON describes, so it works against either director across the atoms-JSON
/// seam. `package_json` MUST be valid JSON; pass `"null"` for the honest empty
/// state.
pub fn render_scene_html(package_json: &str) -> String {
    let payload = scene_payload_json(package_json);
    let injected = format!("window.__SCENE_PACKAGE__ = {payload};");
    SCENE_TEMPLATE
        .replacen(PAYLOAD_MARKER, &injected, 1)
        .replacen(BUNDLE_MARKER, SCENE_BUNDLE, 1)
}

/// Typed convenience: render the scene page for a [`ScenePackageV2`] produced by
/// Lane A. Serializes the package, then delegates to [`render_scene_html`].
///
/// Lane C calls this when it has a typed package in hand; the string form is
/// for callers that already serialized (or that hold a Python-produced
/// package). Returns the package's JSON serialization error if it cannot be
/// serialized (it always can for a well-formed package).
pub fn render_scene(package: &ScenePackageV2) -> Result<String, serde_json::Error> {
    let json = serde_json::to_string(package)?;
    Ok(render_scene_html(&json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene_os_core::{
        AtomLifecycle, ChromeBinding, ProjectionBinding, SceneAtom, ScenePackageV2, SceneRelation,
    };
    use std::collections::BTreeMap;

    fn atom(id: &str, kind: &str, label: &str) -> SceneAtom {
        SceneAtom {
            id: id.to_string(),
            kind: kind.to_string(),
            label: Some(label.to_string()),
            position: None,
            weight: None,
            color: None,
            opacity: None,
            glyph: None,
            scale: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        }
    }

    fn relation(id: &str, source: &str, target: &str) -> SceneRelation {
        SceneRelation {
            id: id.to_string(),
            source_id: source.to_string(),
            target_id: target.to_string(),
            kind: "supports".to_string(),
            weight: None,
            color: None,
            opacity: None,
            glyph: None,
            lifecycle: AtomLifecycle::Present,
            metadata: BTreeMap::new(),
            source_refs: Vec::new(),
        }
    }

    fn sample_package() -> ScenePackageV2 {
        ScenePackageV2 {
            version: "scene-package-v2".to_string(),
            id: "pkg-1".to_string(),
            manifest_ref: "manifest-1".to_string(),
            atoms: vec![
                atom("a", "claim", "Conclusion"),
                atom("b", "evidence", "Supporting source"),
            ],
            relations: vec![relation("b->a", "b", "a")],
            projection: ProjectionBinding {
                id: "tree_hierarchy".to_string(),
                params: BTreeMap::new(),
            },
            chrome: ChromeBinding {
                id: "document_rail".to_string(),
                params: BTreeMap::new(),
            },
            actions: Vec::new(),
            transitions: None,
            terminal_state: None,
            provenance: BTreeMap::new(),
        }
    }

    #[test]
    fn render_injects_payload_and_bundle_and_consumes_markers() {
        let html = render_scene(&sample_package()).expect("render");
        assert!(
            html.contains("window.__SCENE_PACKAGE__ = {"),
            "payload injected"
        );
        assert!(
            !html.contains("// __SCENE_PACKAGE__"),
            "payload marker consumed"
        );
        assert!(
            !html.contains("/*__SCENE_OS_BUNDLE__*/"),
            "bundle marker consumed"
        );
        assert!(html.contains("tree_hierarchy"), "projection id present");
        assert!(html.contains("Supporting source"), "atom label present");
        // The renderer bundle is inlined (self-contained): its IIFE prologue and
        // a known palette constant are present.
        assert!(html.contains("use strict"), "bundle inlined");
        assert!(html.contains("SceneOS"), "renderer API present in bundle");
    }

    #[test]
    fn payload_is_valid_json_after_escaping() {
        let html = render_scene(&sample_package()).expect("render");
        // Extract the injected literal and confirm it re-parses as JSON.
        let start =
            html.find("window.__SCENE_PACKAGE__ = ").unwrap() + "window.__SCENE_PACKAGE__ = ".len();
        let rest = &html[start..];
        let end = rest.find(";\n").unwrap();
        let json = &rest[..end];
        let parsed: serde_json::Value =
            serde_json::from_str(json).expect("escaped payload must remain valid JSON");
        assert_eq!(parsed["projection"]["id"], "tree_hierarchy");
        assert_eq!(parsed["atoms"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn script_breakout_is_neutralized() {
        // An atom label that tries to close the script tag and inject markup.
        let mut malicious = sample_package();
        malicious.atoms[0].label = Some("</script><img src=x onerror=alert(1)>".to_string());
        let html = render_scene(&malicious).expect("render");
        assert!(
            !html.contains("</script><img"),
            "raw script breakout must not survive into the page"
        );
        assert!(
            html.contains("\\u003c/script\\u003e\\u003cimg"),
            "the label is present, but escaped"
        );
    }

    #[test]
    fn null_package_renders_a_valid_empty_page() {
        let html = render_scene_html("null");
        assert!(html.contains("window.__SCENE_PACKAGE__ = null;"));
        assert!(html.contains("<!doctype html>"));
        assert!(
            !html.contains("/*__SCENE_OS_BUNDLE__*/"),
            "bundle still injected"
        );
        // Empty state scaffold is present so the bundle can show it honestly.
        assert!(html.contains("scene-empty"));
    }

    #[test]
    fn round_trip_string_and_typed_apis_agree() {
        let pkg = sample_package();
        let from_typed = render_scene(&pkg).expect("typed");
        let from_string = render_scene_html(&serde_json::to_string(&pkg).unwrap());
        assert_eq!(from_typed, from_string);
    }
}
