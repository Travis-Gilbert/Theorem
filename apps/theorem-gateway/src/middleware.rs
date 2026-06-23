//! HTTP-edge middleware: CORS and client-IP resolution.
//!
//! The `/graphql` endpoint is the only browser boundary, so CORS is the gate
//! that decides which web origins may call it. Client IP is resolved from the
//! proxy headers Railway sets (`X-Forwarded-For` / `X-Real-IP`), falling back
//! to the socket peer, and is used to key the per-IP rate limiter.

use std::net::SocketAddr;

use axum::http::{header, HeaderMap, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

/// Build the CORS layer from the configured allow-list. An empty list means
/// "permissive" (dev only) with a startup warning; a populated list restricts
/// to exactly those origins.
pub fn build_cors_layer(origins: &[String]) -> CorsLayer {
    let methods = [Method::GET, Method::POST, Method::OPTIONS];
    let headers = [header::CONTENT_TYPE, header::AUTHORIZATION];

    if origins.is_empty() {
        tracing::warn!(
            "CORS_ALLOW_ORIGINS unset: allowing any origin (dev default). \
             Set it to the website origin in production."
        );
        return CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(methods)
            .allow_headers(headers);
    }

    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| match origin.parse::<HeaderValue>() {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::warn!("CORS origin ignored (unparseable): {origin}");
                None
            }
        })
        .collect();

    tracing::info!("CORS restricted to {} origin(s)", parsed.len());
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(parsed))
        .allow_methods(methods)
        .allow_headers(headers)
}

/// Resolve the originating client IP for rate limiting. Prefers the first hop
/// in `X-Forwarded-For`, then `X-Real-IP`, then the TCP peer address.
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> String {
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        if let Some(first) = forwarded.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|value| value.to_str().ok()) {
        let ip = real_ip.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    peer.ip().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> SocketAddr {
        "203.0.113.7:5555".parse().unwrap()
    }

    #[test]
    fn prefers_forwarded_for_first_hop() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.9, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&headers, peer()), "198.51.100.9");
    }

    #[test]
    fn falls_back_to_real_ip_then_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "192.0.2.44".parse().unwrap());
        assert_eq!(client_ip(&headers, peer()), "192.0.2.44");

        let empty = HeaderMap::new();
        assert_eq!(client_ip(&empty, peer()), "203.0.113.7");
    }
}
