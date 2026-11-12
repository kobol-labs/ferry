use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, PartialEq, Eq)]
pub enum ContentSegment {
    Text(String),
    Code(String),
}

/// Segments a tool output into plain text chunks and protected markdown code blocks.
/// Supports multi-backtick nested fences (e.g. 4+ backticks enclosing 3 backticks).
pub fn segment_markdown_code_blocks(input: &str) -> Vec<ContentSegment> {
    let mut segments = Vec::new();
    let mut current_chunk = Vec::new();
    let mut in_code_block = false;
    let mut fence_len = 0;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            let backtick_count = trimmed.chars().take_while(|&c| c == '`').count();
            if in_code_block {
                if backtick_count >= fence_len {
                    current_chunk.push(line);
                    segments.push(ContentSegment::Code(current_chunk.join("\n")));
                    current_chunk.clear();
                    in_code_block = false;
                    fence_len = 0;
                } else {
                    current_chunk.push(line);
                }
            } else {
                if !current_chunk.is_empty() {
                    segments.push(ContentSegment::Text(current_chunk.join("\n")));
                    current_chunk.clear();
                }
                current_chunk.push(line);
                in_code_block = true;
                fence_len = backtick_count;
            }
        } else {
            current_chunk.push(line);
        }
    }

    if !current_chunk.is_empty() {
        let content = current_chunk.join("\n");
        if in_code_block {
            segments.push(ContentSegment::Code(content));
        } else {
            segments.push(ContentSegment::Text(content));
        }
    }

    segments
}

/// Extracts critical anchor entities from text (file paths, URLs, ports, imperatives)
pub fn extract_critical_anchors(text: &str) -> Vec<String> {
    extract_anchors_internal(text, true)
}

/// Extracts technical/security anchors from tool outputs without matching common English prose words
/// ("cannot", "not", "never", "must", "high") that appear in source code comments and cause false vetoes.
pub fn extract_tool_anchors(text: &str) -> Vec<String> {
    extract_anchors_internal(text, false)
}

pub fn extract_anchors_internal(text: &str, include_prose_imperatives: bool) -> Vec<String> {
    let mut anchors = Vec::new();

    for token in text.split_whitespace() {
        let clean = token.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')' || c == '"' || c == '\'' || c == '[' || c == ']' || c == '<' || c == '>');
        let clean = clean.strip_suffix('.').unwrap_or(clean);
        let clean = clean.strip_suffix(':').unwrap_or(clean);

        if clean.len() < 3 {
            continue;
        }

        let upper = clean.to_ascii_uppercase();

        // 1. URLs
        if clean.starts_with("http://") || clean.starts_with("https://") {
            anchors.push(clean.to_string());
            continue;
        }

        // 2. CVE & Advisory identifiers (e.g., CVE-2024-1234, GHSA-xxxx-xxxx)
        if upper.starts_with("CVE-") || upper.starts_with("GHSA-") {
            anchors.push(clean.to_string());
            continue;
        }

        // 3. File paths with directories and API endpoints
        if (clean.starts_with('/') && clean.contains('/'))
            || ((clean.contains('/') || clean.contains('\\'))
                && (clean.ends_with(".go")
                    || clean.ends_with(".rs")
                    || clean.ends_with(".py")
                    || clean.ends_with(".js")
                    || clean.ends_with(".ts")
                    || clean.ends_with(".json")
                    || clean.ends_with(".toml")
                    || clean.ends_with(".yaml")
                    || clean.ends_with(".yml")
                    || clean.ends_with(".sql")
                    || clean.ends_with(".sh")
                    || clean.starts_with("api/")
                    || clean.starts_with("v1/")
                    || clean.starts_with("v2/")))
        {
            anchors.push(clean.to_string());
            continue;
        }

        // 4. Critical standalone configuration files
        if clean == "Dockerfile" || clean == "Makefile" || clean == "Cargo.toml" || clean == "go.mod" || clean == "package.json" || clean == ".env" {
            anchors.push(clean.to_string());
            continue;
        }

        // 5. Host:Port, IPv4 addresses, and Port bindings (:8080, 8080/tcp)
        let is_port_binding = clean.starts_with(':') && clean[1..].chars().all(|c| c.is_ascii_digit()) && clean.len() <= 6;
        let is_port_proto = clean.contains("/tcp") || clean.contains("/udp");
        let is_ipv4 = clean.split('.').count() == 4 && clean.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let is_host_port = clean.contains(':')
            && clean.split(':').count() == 2
            && {
                let port = clean.split(':').nth(1).unwrap();
                !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit())
            };

        if is_port_binding || is_port_proto || is_ipv4 || is_host_port {
            anchors.push(clean.to_string());
            continue;
        }

        // 6. Security keywords and imperatives
        if upper == "CRITICAL" || upper == "CVSS" {
            anchors.push(clean.to_string());
            continue;
        }

        if include_prose_imperatives
            && (upper == "HIGH"
                || upper == "NOT"
                || upper == "NEVER"
                || upper == "MUST"
                || upper == "DENY"
                || upper == "DON'T"
                || upper == "DONT"
                || upper == "DO_NOT"
                || upper == "CANNOT"
                || upper == "FORBIDDEN"
                || upper == "RESTRICTED"
                || upper == "PROHIBITED"
                || upper == "BYPASS")
        {
            anchors.push(clean.to_string());
            continue;
        }
    }

    anchors
}

pub fn extract_compact_tool_arguments(args_json: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(args_json) {
        if let Some(obj) = val.as_object() {
            for key in &["cmd", "command", "CommandLine", "url", "path", "endpoint", "query", "script"] {
                if let Some(v) = obj.get(*key).and_then(|v| v.as_str()) {
                    let snippet: String = v.chars().take(120).collect();
                    return format!("{}=\"{}\"", key, snippet.trim());
                }
            }
        }
    }
    let snippet: String = args_json.chars().take(100).collect();
    snippet.trim().to_string()
}

pub fn collect_discovered_entities(
    text: &str,
    endpoints: &mut BTreeSet<String>,
    auth: &mut BTreeSet<String>,
    anomalies: &mut BTreeSet<String>,
) {
    for token in text.split_whitespace() {
        let clean = token.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')' || c == '"' || c == '\'' || c == '[' || c == ']' || c == '<' || c == '>' || c == '{' || c == '}');
        let upper = clean.to_ascii_uppercase();

        // 1. Endpoints & URLs
        if (clean.starts_with("/api/") || clean.starts_with("http://") || clean.starts_with("https://")) && clean.len() < 80 {
            endpoints.insert(clean.to_string());
            if endpoints.len() > 15 {
                let first = endpoints.iter().next().cloned().unwrap();
                endpoints.remove(&first);
            }
        }

        // 2. Auth artifacts: tokens, cookies, headers
        if (clean.starts_with("sps_session") || clean.starts_with("token_version") || clean.starts_with("eyJ") || upper.starts_with("BEARER")) && clean.len() <= 500 {
            auth.insert(clean.to_string());
            if auth.len() > 10 {
                let first = auth.iter().next().cloned().unwrap();
                auth.remove(&first);
            }
        }

        // 3. Security Finding IDs (CVE, CWE, GHSA)
        if (upper.starts_with("CVE-") || upper.starts_with("CWE-") || upper.starts_with("GHSA-")) && clean.len() < 40 {
            anomalies.insert(clean.to_string());
            if anomalies.len() > 10 {
                let first = anomalies.iter().next().cloned().unwrap();
                anomalies.remove(&first);
            }
        }
    }
}
