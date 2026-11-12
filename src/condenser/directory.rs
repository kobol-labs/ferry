pub const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
    "mix.lock",
    "flake.lock",
];

pub const LOCKFILE_EXTENSIONS: &[&str] = &[
    ".min.js",
    ".min.css",
    ".map",
    ".bundle.js",
    ".bundle.css",
];

pub const DIR_SHIELD_PATTERNS: &[(&str, &str)] = &[
    ("node_modules/", "node_modules/"),
    (".git/", ".git/"),
    ("target/debug/", "target/debug/"),
    ("target/release/", "target/release/"),
    ("__pycache__/", "__pycache__/"),
    (".venv/", ".venv/"),
    ("/venv/", "venv/"),
    ("vendor/bundle/", "vendor/bundle/"),
    (".cache/", ".cache/"),
];

pub fn is_directory_listing_line(line: &str, category: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains(category) {
        return false;
    }
    if trimmed.contains(';') || trimmed.contains('{') || trimmed.contains('}') || trimmed.contains('=') {
        return false;
    }
    if trimmed.starts_with("import ") || trimmed.starts_with("from ")
        || trimmed.starts_with("def ") || trimmed.starts_with("class ")
        || trimmed.starts_with("const ") || trimmed.starts_with("let ")
        || trimmed.starts_with("var ") || trimmed.starts_with("//")
        || trimmed.starts_with("/*") || trimmed.starts_with('*') || trimmed.starts_with('#') {
        return false;
    }
    true
}

/// Prunes verbose diff bodies of lockfiles and generated asset bundles in git diff output.
pub fn prune_git_diff_lockfiles(input: &str) -> (String, usize) {
    if !input.contains("diff --git ") {
        return (input.to_string(), 0);
    }

    let mut out = String::with_capacity(input.len());
    let mut pruned_count = 0;

    let chunks: Vec<&str> = input.split("diff --git ").collect();
    if chunks.is_empty() {
        return (input.to_string(), 0);
    }

    out.push_str(chunks[0]);

    for chunk in &chunks[1..] {
        out.push_str("diff --git ");
        let first_line = chunk.lines().next().unwrap_or("");

        let is_target_file = LOCKFILE_NAMES.iter().any(|name| first_line.contains(&format!("/{name}")) || first_line.ends_with(name))
            || LOCKFILE_EXTENSIONS.iter().any(|ext| first_line.contains(ext));

        if is_target_file {
            let mut adds = 0;
            let mut dels = 0;
            for line in chunk.lines() {
                if line.starts_with('+') && !line.starts_with("+++") {
                    adds += 1;
                } else if line.starts_with('-') && !line.starts_with("---") {
                    dels += 1;
                }
            }

            if (adds + dels) > 10 {
                for line in chunk.lines() {
                    out.push_str(line);
                    out.push('\n');
                    if line.starts_with("+++") || line.starts_with("@@") {
                        break;
                    }
                }
                out.push_str(&format!(
                    "[... Ferry Proxy: lockfile/asset diff suppressed (+{adds} lines, -{dels} lines) ...]\n"
                ));
                pruned_count += 1;
                continue;
            }
        }

        out.push_str(chunk);
    }

    (out, pruned_count)
}

/// Folds long runs of file traversals into dependency or cache directories.
pub fn shield_directory_traversals(input: &str) -> (String, usize) {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < 6 {
        return (input.to_string(), 0);
    }

    let mut out = Vec::new();
    let mut shielded_count = 0;
    let mut idx = 0;

    while idx < lines.len() {
        let line = lines[idx];
        let mut matched_cat = None;

        for (pattern, cat) in DIR_SHIELD_PATTERNS {
            if is_directory_listing_line(line, pattern) {
                matched_cat = Some(*cat);
                break;
            }
        }

        if let Some(cat) = matched_cat {
            let start_run = idx;
            while idx < lines.len() && is_directory_listing_line(lines[idx], cat) {
                idx += 1;
            }
            let run_count = idx - start_run;
            if run_count >= 4 {
                out.push(format!("[... Ferry Proxy: {run_count} directory entries in {cat} suppressed ...]"));
                shielded_count += run_count;
            } else {
                for i in start_run..idx {
                    out.push(lines[i].to_string());
                }
            }
        } else {
            out.push(line.to_string());
            idx += 1;
        }
    }

    (out.join("\n"), shielded_count)
}
