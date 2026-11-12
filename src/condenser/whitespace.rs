pub const COMMON_CODE_EXTS: &[&str] = &[
    ".rs", ".py", ".js", ".ts", ".go", ".json", ".toml", ".yaml", ".yml",
    ".md", ".txt", ".html", ".css", ".sh", ".c", ".cpp", ".h", ".sql"
];

pub fn is_base64_or_hex_payload(token: &str) -> bool {
    if token.len() < 120 {
        return false;
    }
    // Never suppress JWT tokens, API keys, or authentication headers
    if token.starts_with("eyJ")
        || token.starts_with("sk-")
        || token.starts_with("ghp_")
        || token.starts_with("gho_")
        || token.starts_with("glpat-")
        || token.starts_with("xoxb-")
    {
        return false;
    }
    if token.starts_with('/') || token.starts_with("./") || token.starts_with("../") || token.starts_with("http://") || token.starts_with("https://") {
        return false;
    }
    if COMMON_CODE_EXTS.iter().any(|ext| token.ends_with(ext)) {
        return false;
    }
    if token.matches('/').count() >= 2 {
        return false;
    }
    true
}

/// Strips ANSI terminal escape codes (e.g. \x1b[32m or \x1b[0m) without external regex crate
pub fn strip_ansi_codes(input: &str) -> (String, bool) {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut modified = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                modified = true;
                // Consume characters until terminator [a-zA-Z] or end of string
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }

    (out, modified)
}

/// Resolves `\r` carriage returns (keeps final line overwrite) and removes progress bars
pub fn strip_progress_artifacts(input: &str) -> (String, bool) {
    let mut clean_lines = Vec::new();
    let mut modified = false;

    for line in input.lines() {
        let (effective_line, had_cr) = if line.contains('\r') {
            (
                line.split('\r').filter(|s| !s.trim().is_empty()).last().unwrap_or(""),
                true,
            )
        } else {
            (line, false)
        };

        if had_cr {
            modified = true;
        }

        let trimmed = effective_line.trim();
        let is_progress = (trimmed.contains('[') && trimmed.contains(']') && trimmed.contains('%') && (trimmed.contains('=') || trimmed.contains('>')))
            || (trimmed.contains('|') && (trimmed.contains('█') || trimmed.contains('#')) && trimmed.contains('%'))
            || (trimmed.starts_with("Downloading ") && trimmed.ends_with('%'))
            || (trimmed.starts_with("Fetch: ") && trimmed.ends_with('%'));

        if is_progress {
            modified = true;
        } else {
            clean_lines.push(effective_line);
        }
    }

    (clean_lines.join("\n"), modified)
}

/// Collapses runs of consecutive identical lines into a single line with `(repeated N times)`
/// Safety invariants:
/// 1. Only deduplicate if repeat_count >= 3 (never for just 2 repetitions).
/// 2. Line must have trimmed length >= 12 and contain alphanumeric characters.
///    This strictly avoids mangling nested code closures (e.g. `}`, `);`, `---`, `***`).
pub fn deduplicate_lines(input: &str) -> (String, usize) {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() <= 2 {
        return (input.to_string(), 0);
    }

    let mut result = Vec::new();
    let mut current_line = lines[0];
    let mut repeat_count = 1;
    let mut dedup_count = 0;

    let can_dedup = |line: &str| -> bool {
        let trimmed = line.trim();
        trimmed.len() >= 12 && trimmed.chars().any(|c| c.is_alphanumeric())
    };

    for &line in &lines[1..] {
        if line == current_line && !line.trim().is_empty() {
            repeat_count += 1;
        } else {
            result.push(current_line.to_string());
            if repeat_count >= 3 && can_dedup(current_line) {
                result.push(format!("    [... repeated {} times ...]", repeat_count - 1));
                dedup_count += 1;
            } else if repeat_count > 1 {
                for _ in 1..repeat_count {
                    result.push(current_line.to_string());
                }
            }
            current_line = line;
            repeat_count = 1;
        }
    }

    result.push(current_line.to_string());
    if repeat_count >= 3 && can_dedup(current_line) {
        result.push(format!("    [... repeated {} times ...]", repeat_count - 1));
        dedup_count += 1;
    } else if repeat_count > 1 {
        for _ in 1..repeat_count {
            result.push(current_line.to_string());
        }
    }

    (result.join("\n"), dedup_count)
}

/// Collapses 3+ consecutive newlines into 2, and trims trailing whitespace per line
pub fn collapse_excess_whitespace(input: &str) -> (String, bool) {
    let mut out = String::with_capacity(input.len());
    let mut consecutive_newlines = 0;
    let mut modified = false;

    for line in input.lines() {
        let trimmed_line = line.trim_end();
        if trimmed_line.len() != line.len() {
            modified = true;
        }

        if trimmed_line.is_empty() {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                out.push('\n');
            } else {
                modified = true;
            }
        } else {
            consecutive_newlines = 0;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(trimmed_line);
        }
    }

    (out, modified)
}

/// Replaces large inline base64 payloads and continuous hex blobs with size banners.
pub fn suppress_base64_and_hex_blobs(input: &str) -> (String, usize) {
    if input.len() < 120 {
        return (input.to_string(), 0);
    }

    let mut out = String::with_capacity(input.len());
    let mut suppressed_count = 0;

    for line in input.lines() {
        if line.len() < 120 {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let mut processed_line = String::with_capacity(line.len());
        let mut remainder = line;

        while let Some(data_idx) = remainder.find("data:") {
            if let Some(semi_idx) = remainder[data_idx..].find(";base64,") {
                if semi_idx < 64 && !remainder[data_idx..data_idx + semi_idx].contains(' ') {
                    let base64_start = data_idx + semi_idx + 8;
                    let base64_len = remainder[base64_start..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
                        .count();

                    if base64_len >= 80 {
                        processed_line.push_str(&remainder[..base64_start]);
                        processed_line.push_str(&format!("[... Ferry Proxy: base64 payload ({base64_len} chars) suppressed ...]"));
                        remainder = &remainder[base64_start + base64_len..];
                        suppressed_count += 1;
                        continue;
                    }
                }
            }
            processed_line.push_str(&remainder[..data_idx + 5]);
            remainder = &remainder[data_idx + 5..];
        }
        processed_line.push_str(remainder);

        let mut line_tokens = Vec::new();
        let mut current_token = String::new();
        let is_b64_char = |c: char| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=';

        for c in processed_line.chars() {
            if is_b64_char(c) {
                current_token.push(c);
            } else {
                if is_base64_or_hex_payload(&current_token) {
                    line_tokens.push(format!("[... Ferry Proxy: base64/hex blob ({} chars) suppressed ...]", current_token.len()));
                    suppressed_count += 1;
                } else {
                    line_tokens.push(current_token.clone());
                }
                current_token.clear();
                line_tokens.push(c.to_string());
            }
        }
        if is_base64_or_hex_payload(&current_token) {
            line_tokens.push(format!("[... Ferry Proxy: base64/hex blob ({} chars) suppressed ...]", current_token.len()));
            suppressed_count += 1;
        } else {
            line_tokens.push(current_token);
        }

        out.push_str(&line_tokens.concat());
        out.push('\n');
    }

    if out.ends_with('\n') && !input.ends_with('\n') {
        out.pop();
    }

    (out, suppressed_count)
}
