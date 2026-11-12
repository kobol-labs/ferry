pub fn safe_floor_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub fn safe_ceil_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while index < s.len() && !s.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub const DIAGNOSTIC_SIGNALS: &[&str] = &[
    "error:",
    "error[",
    "failed",
    "failure",
    "exception",
    "panic:",
    "panicked",
    "fatal:",
    "vulnerability",
    "vulnerabilities",
    "cwe-",
    "cve-",
    "ghsa-",
    "assert",
    "traceback",
    "syntaxerror",
    "typeerror",
    "referenceerror",
    "status 500",
    "status 502",
    "status 503",
    "status 504",
    "status 400",
    "status 401",
    "status 403",
    "status 404",
    "status 422",
    "status 429",
    "bad request",
    "not found",
    "internal server error",
    "bad gateway",
    "service unavailable",
    "gateway timeout",
    "unauthorized",
    "forbidden",
    "access denied",
    "permission denied",
    "connection refused",
    "timed out",
    "timeout",
    "no such file",
    "command not found",
    "cannot find module",
    "module not found",
    "invalid",
    "unrecognized",
    "malformed",
    "corrupt",
    "warning:",
    "warn:",
    "critical:",
    "rejected",
    "expired",
    "sigsegv",
    "segmentation fault",
    "sql syntax",
    "sqlite3.",
    "syntax error near",
    "unterminated quotation",
    "sqli",
    "xss",
    "csrf",
    "idor",
    "ssrf",
    "token_version",
    "jwt",
    "bearer",
    "leak",
    "bypass",
    "unhandled",
    "uncaught",
    "nullpointer",
    "stack overflow",
];

/// Strips all inline `<think>...</think>` tags from assistant text
pub fn strip_think_tags(text: &str) -> Option<String> {
    let open_tag = "<think>";
    let close_tag = "</think>";

    if !text.contains(open_tag) {
        return None;
    }

    let mut current = text.to_string();
    let mut modified = false;

    while let Some(start) = current.find(open_tag) {
        if let Some(end_offset) = current[start..].find(close_tag) {
            let end = start + end_offset + close_tag.len();
            let before = &current[..start];
            let after = &current[end..];
            let had_space = before.ends_with(char::is_whitespace) || after.starts_with(char::is_whitespace);
            let sep = if had_space && !before.is_empty() && !after.is_empty() { " " } else { "" };
            current = format!("{}{}{}", before.trim_end(), sep, after.trim_start());
            modified = true;
        } else {
            // Unclosed <think> tag: strip from <think> to end of text
            current = current[..start].trim_end().to_string();
            modified = true;
            break;
        }
    }

    if modified {
        Some(current.trim().to_string())
    } else {
        None
    }
}

/// Scans lines for diagnostic error signals and returns line index of the highest density signal region
pub fn find_diagnostic_signal_index(lines: &[&str], start_idx: usize, end_idx: usize) -> Option<usize> {
    let regions = find_diagnostic_signal_regions(lines, start_idx, end_idx);
    regions.first().map(|(start, end)| (start + end) / 2)
}

/// Scans lines for diagnostic error signals and returns up to 3 non-overlapping signal windows (start_line, end_line)
pub fn find_diagnostic_signal_regions(lines: &[&str], start_idx: usize, end_idx: usize) -> Vec<(usize, usize)> {
    if start_idx >= end_idx || lines.is_empty() {
        return Vec::new();
    }
    let mut signal_line_indices = Vec::new();
    for i in start_idx..end_idx.min(lines.len()) {
        let line_lower = lines[i].to_ascii_lowercase();
        let has_signal = DIAGNOSTIC_SIGNALS.iter().any(|sig| line_lower.contains(sig));
        if has_signal {
            signal_line_indices.push(i);
        }
    }
    if signal_line_indices.is_empty() {
        return Vec::new();
    }

    let mut regions = Vec::new();
    let mut cur_start = signal_line_indices[0].saturating_sub(2).max(start_idx);
    let mut cur_end = (signal_line_indices[0] + 5).min(end_idx);

    for &idx in &signal_line_indices[1..] {
        let win_start = idx.saturating_sub(2).max(start_idx);
        let win_end = (idx + 5).min(end_idx);
        if win_start <= cur_end + 2 {
            if win_end - cur_start <= 12 {
                cur_end = cur_end.max(win_end);
            } else {
                regions.push((cur_start, cur_end));
                cur_start = win_start;
                cur_end = win_end;
                if regions.len() >= 3 {
                    break;
                }
            }
        } else {
            regions.push((cur_start, cur_end));
            cur_start = win_start;
            cur_end = win_end;
            if regions.len() >= 3 {
                break;
            }
        }
    }
    if regions.len() < 3 && cur_start < cur_end {
        regions.push((cur_start, cur_end));
    }
    regions
}

/// Truncates long tool content keeping head and tail lines with omission indicator.
/// Guarantees 100% panic-freedom on multi-byte UTF-8 inputs.
pub fn truncate_tool_content(text: &str, head_lines: usize, tail_lines: usize) -> String {
    let mut dummy = 0;
    truncate_tool_content_smart(text, head_lines, tail_lines, &mut dummy)
}

/// Signal-aware smart truncation that preserves error diagnostic regions in the middle of long outputs.
pub fn truncate_tool_content_smart(
    text: &str,
    head_lines: usize,
    tail_lines: usize,
    signals_extracted: &mut usize,
) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head_lines + tail_lines {
        // Line count is small, do byte/char boundary truncation safely on UTF-8 boundaries.
        // First check if the middle portion contains diagnostic/security signals before blind truncation!
        let head_target = (head_lines * 80).clamp(600, 1800).min(text.len() / 3);
        let tail_target = (tail_lines * 80).clamp(600, 1800).min(text.len() / 3);
        let head_idx = safe_floor_char_boundary(text, head_target);
        let tail_start = text.len().saturating_sub(tail_target);
        let tail_idx = safe_ceil_char_boundary(text, tail_start);

        if head_idx < tail_idx {
            let mid_slice = &text[head_idx..tail_idx];
            let mid_lower = mid_slice.to_ascii_lowercase();
            for sig in DIAGNOSTIC_SIGNALS {
                if let Some(pos_in_mid) = mid_lower.find(sig) {
                    *signals_extracted += 1;
                    let abs_pos = head_idx + pos_in_mid;
                    let sig_start = safe_floor_char_boundary(text, abs_pos.saturating_sub(120).max(head_idx));
                    let sig_end = safe_ceil_char_boundary(text, (abs_pos + sig.len() + 200).min(tail_idx));

                    let head_part = &text[..head_idx];
                    let signal_part = &text[sig_start..sig_end];
                    let tail_part = &text[tail_idx..];

                    let dropped_before = sig_start.saturating_sub(head_idx);
                    let dropped_after = tail_idx.saturating_sub(sig_end);

                    let mut out = head_part.to_string();
                    if dropped_before > 0 {
                        out.push_str(&format!("\n\n[... Ferry Proxy: truncated {dropped_before} characters of passing output ...]\n\n"));
                    }
                    out.push_str(signal_part);
                    if dropped_after > 0 {
                        out.push_str(&format!("\n\n[... Ferry Proxy: truncated {dropped_after} characters ...]\n\n"));
                    }
                    out.push_str(tail_part);
                    return out;
                }
            }
        }

        let head_str = &text[..head_idx];
        let tail_str = &text[tail_idx..];
        let dropped = text.len().saturating_sub(head_str.len() + tail_str.len());

        return format!(
            "{}\n\n[... Ferry Proxy: truncated {} characters of earlier tool output ...]\n\n{}",
            head_str,
            dropped,
            tail_str
        );
    }

    // Multi-line smart truncation with multi-region signal preservation
    let mid_start = head_lines;
    let mid_end = lines.len().saturating_sub(tail_lines);
    let signal_regions = find_diagnostic_signal_regions(&lines, mid_start, mid_end);

    if !signal_regions.is_empty() {
        *signals_extracted += signal_regions.len();
        let head_part = lines[..head_lines].join("\n");
        let tail_part = lines[lines.len() - tail_lines..].join("\n");

        let mut out = head_part;
        let mut last_pos = head_lines;

        for (r_start, r_end) in signal_regions {
            let dropped = r_start.saturating_sub(last_pos);
            if dropped > 0 {
                out.push_str(&format!("\n\n[... Ferry Proxy: truncated {dropped} lines of passing logs ...]\n\n"));
            } else {
                out.push('\n');
            }
            out.push_str(&lines[r_start..r_end].join("\n"));
            last_pos = r_end;
        }

        let final_dropped = mid_end.saturating_sub(last_pos);
        if final_dropped > 0 {
            out.push_str(&format!("\n\n[... Ferry Proxy: truncated {final_dropped} lines ...]\n\n"));
        } else {
            out.push('\n');
        }
        out.push_str(&tail_part);
        return out;
    }

    let head_part = lines[..head_lines].join("\n");
    let tail_part = lines[lines.len() - tail_lines..].join("\n");
    let dropped_lines = lines.len() - head_lines - tail_lines;
    let dropped_chars = text.len().saturating_sub(head_part.len() + tail_part.len());

    format!(
        "{head_part}\n\n[... Ferry Proxy: truncated {dropped_lines} lines ({dropped_chars} characters) of earlier tool output ...]\n\n{tail_part}"
    )
}
