use axum::http::{header, HeaderMap, HeaderName, HeaderValue};

pub const CORS_ALLOW_METHODS: &str = "GET, POST, OPTIONS";
pub const CORS_ALLOW_HEADERS: &str = "content-type, authorization, x-api-key, x-ferry-condense, x-ferry-route, x-ferry-cache, x-ferry-queue-timeout-ms, x-ferry-wait-ms, x-ferry-session-id, x-session-id, x-agent-id";
pub const CORS_EXPOSE_HEADERS: &str = "x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover, x-ferry-cache, x-ferry-slot, x-ferry-affinity, x-ferry-session-id";

/// Constant-time comparison of two byte slices without early return on length mismatch.
/// Iterates across `max(expected.len(), provided.len())` positions and uses `black_box`
/// to prevent the compiler from short-circuiting on the first differing byte or length mismatch.
pub fn constant_time_eq(expected: &[u8], provided: &[u8]) -> bool {
    let max_len = expected.len().max(provided.len());
    let mut diff: usize = expected.len() ^ provided.len();
    for i in 0..max_len {
        let a = expected.get(i).copied().unwrap_or(0);
        let b = provided.get(i).copied().unwrap_or(0);
        diff = std::hint::black_box(diff | ((a ^ b) as usize));
    }
    std::hint::black_box(diff) == 0
}

/// Extract an inbound API key from `Authorization: Bearer <token>` (case-insensitive scheme)
/// or, when `Authorization` is absent, from `x-api-key: <token>`.
pub fn extract_inbound_api_key(headers: &HeaderMap) -> Option<&str> {
    if let Some(raw_auth) = headers.get(header::AUTHORIZATION) {
        let auth = raw_auth.to_str().ok()?.trim();
        let rest = auth
            .get(..7)
            .filter(|prefix| prefix.eq_ignore_ascii_case("bearer "))
            .map(|_| auth[7..].trim())?;
        return (!rest.is_empty()).then_some(rest);
    }

    if let Some(raw_key) = headers.get("x-api-key") {
        let key = raw_key.to_str().ok()?.trim();
        return (!key.is_empty()).then_some(key);
    }

    None
}

/// Check whether an inbound request is authorized against the configured `server.api_key`.
/// When `expected_api_key` is empty/whitespace, inbound auth is disabled.
pub fn is_authorized(expected_api_key: &str, headers: &HeaderMap) -> bool {
    let expected = expected_api_key.trim();
    if expected.is_empty() {
        return true;
    }

    match extract_inbound_api_key(headers) {
        Some(provided) => constant_time_eq(expected.as_bytes(), provided.as_bytes()),
        None => false,
    }
}

/// Resolve the `Access-Control-Allow-Origin` value for an incoming request `Origin`
/// against `server.cors_allowed_origins`.
/// Returns `None` when the allowlist is empty or the origin is not permitted.
pub fn resolve_cors_origin(
    request_origin: Option<&str>,
    allowed_origins: &[String],
) -> Option<String> {
    if allowed_origins.is_empty() {
        return None;
    }

    if allowed_origins.iter().any(|o| o.trim() == "*") {
        return Some("*".to_string());
    }

    let req_origin = request_origin?.trim().trim_end_matches('/');
    if req_origin.is_empty() {
        return None;
    }

    allowed_origins
        .iter()
        .map(|o| o.trim().trim_end_matches('/'))
        .find(|allowed| !allowed.is_empty() && *allowed == req_origin)
        .map(ToString::to_string)
}

/// Apply or strip CORS headers on an HTTP response according to `server.cors_allowed_origins`.
pub fn apply_cors_headers(
    headers: &mut HeaderMap,
    request_origin: Option<&str>,
    allowed_origins: &[String],
) {
    headers.remove(HeaderName::from_static("access-control-allow-origin"));
    headers.remove(HeaderName::from_static("access-control-allow-methods"));
    headers.remove(HeaderName::from_static("access-control-allow-headers"));
    headers.remove(HeaderName::from_static("access-control-expose-headers"));

    if let Some(allow_origin) = resolve_cors_origin(request_origin, allowed_origins) {
        if let Ok(origin_val) = HeaderValue::from_str(&allow_origin) {
            headers.insert(
                HeaderName::from_static("access-control-allow-origin"),
                origin_val,
            );
            headers.insert(
                HeaderName::from_static("access-control-allow-methods"),
                HeaderValue::from_static(CORS_ALLOW_METHODS),
            );
            headers.insert(
                HeaderName::from_static("access-control-allow-headers"),
                HeaderValue::from_static(CORS_ALLOW_HEADERS),
            );
            headers.insert(
                HeaderName::from_static("access-control-expose-headers"),
                HeaderValue::from_static(CORS_EXPOSE_HEADERS),
            );
            if allow_origin != "*" {
                headers.append(header::VARY, HeaderValue::from_static("Origin"));
            }
        }
    } else if !allowed_origins.is_empty() {
        headers.append(header::VARY, HeaderValue::from_static("Origin"));
    }
}

/// Returns `true` when the bind string targets a loopback interface (`127.0.0.0/8`, `::1`, `localhost`).
pub fn is_loopback_bind(bind: &str) -> bool {
    let trimmed = bind.trim();
    if let Ok(sock) = trimmed.parse::<std::net::SocketAddr>() {
        return sock.ip().is_loopback();
    }
    let host = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        trimmed.rsplit_once(':').map(|(h, _)| h).unwrap_or(trimmed)
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}
