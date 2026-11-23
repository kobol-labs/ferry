use axum::http::{header, HeaderMap, HeaderValue};
use ferry::config::Config;
use ferry::security::{
    apply_cors_headers, constant_time_eq, extract_inbound_api_key, is_authorized, is_loopback_bind,
    resolve_cors_origin,
};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn test_default_bind_is_loopback_and_docker_sets_explicit_bind() {
    let default_cfg = Config::default();
    assert_eq!(default_cfg.server.bind, "127.0.0.1:4000");
    assert!(default_cfg.server.api_key.is_empty());
    assert!(default_cfg.server.cors_allowed_origins.is_empty());

    for filename in ["ferry.toml", "ferry.example.toml"] {
        let raw = std::fs::read_to_string(repo_root().join(filename))
            .unwrap_or_else(|_| panic!("{filename} must exist"));
        let parsed: Config = toml::from_str(&raw).expect("valid config");
        assert_eq!(
            parsed.server.bind, "127.0.0.1:4000",
            "{filename} must default to loopback bind"
        );
        assert!(
            parsed.server.api_key.is_empty(),
            "{filename} must not commit a hardcoded server.api_key"
        );
        assert!(
            parsed.server.cors_allowed_origins.is_empty(),
            "{filename} must disable cross-origin CORS by default"
        );
    }

    let dockerfile = std::fs::read_to_string(repo_root().join("Dockerfile"))
        .expect("Dockerfile must exist");
    assert!(
        dockerfile.contains("ENV FERRY_BIND=0.0.0.0:4000"),
        "Dockerfile must explicitly set FERRY_BIND=0.0.0.0:4000 so exposed container port 4000 remains reachable"
    );
}

#[test]
fn test_server_security_env_overrides() {
    std::env::set_var("FERRY_BIND", "0.0.0.0:4555");
    std::env::set_var("FERRY_SERVER_API_KEY", "test-inbound-secret-42");
    std::env::set_var(
        "FERRY_CORS_ALLOWED_ORIGINS",
        " https://webui.example.com , http://localhost:3000/ ",
    );

    let cfg = Config::load_from_file_or_default("/nonexistent/ferry.toml");

    std::env::remove_var("FERRY_BIND");
    std::env::remove_var("FERRY_SERVER_API_KEY");
    std::env::remove_var("FERRY_CORS_ALLOWED_ORIGINS");

    assert_eq!(cfg.server.bind, "0.0.0.0:4555");
    assert_eq!(cfg.server.api_key, "test-inbound-secret-42");
    assert_eq!(
        cfg.server.cors_allowed_origins,
        vec![
            "https://webui.example.com".to_string(),
            "http://localhost:3000/".to_string()
        ]
    );
}

#[test]
fn test_constant_time_eq_and_inbound_auth_extraction() {
    assert!(constant_time_eq(b"", b""));
    assert!(constant_time_eq(b"super-secret-token", b"super-secret-token"));
    assert!(!constant_time_eq(b"super-secret-token", b"super-secret-tokeN"));
    assert!(!constant_time_eq(b"super-secret-token", b"super-secret"));
    assert!(!constant_time_eq(b"short", b"short-with-extra-suffix"));
    assert!(!constant_time_eq(b"non-empty", b""));
    assert!(!constant_time_eq(b"", b"non-empty"));

    // Disabled when expected key is empty
    let empty_headers = HeaderMap::new();
    assert!(is_authorized("", &empty_headers));
    assert!(is_authorized("   ", &empty_headers));

    // Missing header rejected when key is set
    assert!(!is_authorized("my-secret", &empty_headers));

    // Bearer auth (case-insensitive scheme)
    let mut bearer_headers = HeaderMap::new();
    bearer_headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer my-secret"),
    );
    assert_eq!(extract_inbound_api_key(&bearer_headers), Some("my-secret"));
    assert!(is_authorized("my-secret", &bearer_headers));
    assert!(!is_authorized("other-secret", &bearer_headers));

    let mut lower_bearer = HeaderMap::new();
    lower_bearer.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("bearer my-secret"),
    );
    assert!(is_authorized("my-secret", &lower_bearer));

    // x-api-key fallback when Authorization is absent
    let mut x_api_key_headers = HeaderMap::new();
    x_api_key_headers.insert("x-api-key", HeaderValue::from_static("my-secret"));
    assert!(is_authorized("my-secret", &x_api_key_headers));

    // Wrong Authorization must NOT fall back to x-api-key
    let mut mixed_headers = HeaderMap::new();
    mixed_headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer wrong-key"),
    );
    mixed_headers.insert("x-api-key", HeaderValue::from_static("my-secret"));
    assert!(!is_authorized("my-secret", &mixed_headers));
}

#[test]
fn test_cors_origin_resolution_and_header_application() {
    // Empty allowlist => never emits Access-Control-Allow-Origin
    let empty: Vec<String> = vec![];
    assert_eq!(
        resolve_cors_origin(Some("https://evil.example"), &empty),
        None
    );

    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    apply_cors_headers(&mut headers, Some("https://evil.example"), &empty);
    assert!(headers.get("access-control-allow-origin").is_none());

    // Specific allowlist => exact match (ignoring trailing slash), denies others
    let allowlist = vec![
        "https://webui.example.com/".to_string(),
        "http://localhost:3000".to_string(),
    ];
    assert_eq!(
        resolve_cors_origin(Some("https://webui.example.com"), &allowlist).as_deref(),
        Some("https://webui.example.com")
    );
    assert_eq!(
        resolve_cors_origin(Some("https://webui.example.com.evil.com"), &allowlist),
        None
    );
    assert_eq!(resolve_cors_origin(None, &allowlist), None);

    let mut allowed_resp_headers = HeaderMap::new();
    apply_cors_headers(
        &mut allowed_resp_headers,
        Some("https://webui.example.com"),
        &allowlist,
    );
    assert_eq!(
        allowed_resp_headers
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://webui.example.com")
    );
    assert_eq!(
        allowed_resp_headers
            .get(header::VARY)
            .and_then(|v| v.to_str().ok()),
        Some("Origin")
    );

    // Explicit wildcard opt-in
    let wildcard = vec!["*".to_string()];
    assert_eq!(
        resolve_cors_origin(Some("https://any.example"), &wildcard).as_deref(),
        Some("*")
    );
}

#[test]
fn test_is_loopback_bind() {
    assert!(is_loopback_bind("127.0.0.1:4000"));
    assert!(is_loopback_bind("[::1]:4000"));
    assert!(is_loopback_bind("localhost:4000"));
    assert!(!is_loopback_bind("0.0.0.0:4000"));
    assert!(!is_loopback_bind("[::]:4000"));
    assert!(!is_loopback_bind("192.168.1.50:4000"));
}
