//! Deliverable B: serve the compiled scene page.
//!
//! `GET /scene/{sceneId}` looks the package up in the in-memory store and
//! renders it through `scene-os-web::render_scene`, which injects the package
//! JSON via the `scene_payload_json` escape path (`<`, `>`, `&` -> `\uXXXX`) so
//! a crawled or agent-authored label containing `</script>` cannot break out.
//! An unknown id renders the honest empty state (`null`) with a 404.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};

use crate::AppState;

pub async fn serve_scene(
    State(state): State<AppState>,
    Path(scene_id): Path<String>,
) -> impl IntoResponse {
    match state.scenes.get(&scene_id) {
        Some(package) => match scene_os_web::render_scene(&package) {
            Ok(html) => (StatusCode::OK, Html(html)),
            Err(error) => {
                tracing::error!("scene render failed for {scene_id}: {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html(scene_os_web::render_scene_html("null")),
                )
            }
        },
        None => (
            StatusCode::NOT_FOUND,
            Html(scene_os_web::render_scene_html("null")),
        ),
    }
}
