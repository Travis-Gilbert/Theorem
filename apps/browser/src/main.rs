//! Theorem Browser embedder (Servo) — build-validation entry.
//!
//! This is v2b step 1 of the substrate-native browser. Its only job is to prove
//! that `libservo` builds and links as a Cargo git dependency at the pinned rev
//! (see Cargo.toml) and that our engine-construction wiring compiles against the
//! real Servo API. It does NOT open a window or load a page yet.
//!
//! Why so small: the embedder crate compiles AFTER libservo (~30 min from cold),
//! so any error here costs a full libservo rebuild in CI. Step 1 keeps the API
//! surface minimal (confirmed against servo/components/servo/examples/winit_minimal.rs
//! and ports/servoshell/desktop/app.rs at the pinned rev) to maximize one-shot
//! success and warm the build cache. Step 2 adds the WebView + the
//! `WebViewDelegate::load_web_resource` substrate seam (which calls
//! `theorem_browser_substrate::ingest_loaded_pages`) and iterates on the warm cache.

use servo::{EventLoopWaker, ServoBuilder};

/// Minimal event-loop waker.
///
/// `Servo::spin_event_loop` is driven by the embedder; when Servo needs the
/// embedder to pump the loop it calls `wake()`. A headless build-validation run
/// does not pump a real loop, so this is a no-op. Step 2's windowed/headless
/// runtime will wake an actual loop (winit proxy or a condvar).
#[derive(Clone)]
struct HeadlessWaker;

impl EventLoopWaker for HeadlessWaker {
    fn wake(&self) {}

    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
}

fn main() {
    // Construct the engine with defaults (Opts/Preferences default; only the
    // waker is required). Proves the git-dep builds and the wiring compiles.
    let _servo = ServoBuilder::default()
        .event_loop_waker(Box::new(HeadlessWaker))
        .build();

    println!("theorem-browser: Servo engine constructed (build validation OK)");
}
