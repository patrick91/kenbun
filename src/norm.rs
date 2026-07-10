//! PEP 503 name normalization and PEP 508 name extraction.

/// PEP 503: lowercase, collapse runs of `-`, `_`, `.` into a single `-`.
pub fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = false;
    for c in name.chars() {
        if c == '-' || c == '_' || c == '.' {
            if !prev_sep && !out.is_empty() {
                out.push('-');
            }
            prev_sep = true;
        } else {
            out.push(c.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Split a PEP 508 requirement string into (normalized name, extras, rest).
/// Returns None for empty lines, comments, pip options, and nameless specs.
pub fn split_requirement(spec: &str) -> Option<(String, Vec<String>, String)> {
    let s = spec.trim();
    if s.is_empty() || s.starts_with('#') || s.starts_with('-') {
        return None;
    }
    if !s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let name_end = s
        .char_indices()
        .find(|(_, c)| {
            matches!(
                c,
                '[' | '=' | '<' | '>' | '!' | '~' | ';' | '@' | '(' | ' ' | '\t'
            )
        })
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let name = &s[..name_end];
    let mut rest = &s[name_end..];

    let mut extras = Vec::new();
    let trimmed = rest.trim_start();
    if let Some(inner) = trimmed.strip_prefix('[') {
        if let Some(close) = inner.find(']') {
            extras = inner[..close]
                .split(',')
                .map(|e| e.trim().to_string())
                .filter(|e| !e.is_empty())
                .collect();
            rest = &inner[close + 1..];
        }
    }
    Some((normalize_name(name), extras, rest.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_per_pep503() {
        assert_eq!(normalize_name("FastAPI"), "fastapi");
        assert_eq!(normalize_name("Fast.API"), "fast-api");
        assert_eq!(normalize_name("fast__api"), "fast-api");
        assert_eq!(normalize_name("fastapi-slim"), "fastapi-slim");
    }

    #[test]
    fn splits_requirements() {
        let (name, extras, rest) = split_requirement("fastapi[standard]>=0.115").unwrap();
        assert_eq!(name, "fastapi");
        assert_eq!(extras, vec!["standard"]);
        assert_eq!(rest, ">=0.115");

        let (name, _, _) = split_requirement("FastAPI @ git+https://x").unwrap();
        assert_eq!(name, "fastapi");

        assert!(split_requirement("# comment").is_none());
        assert!(split_requirement("-r other.txt").is_none());
        assert!(split_requirement("").is_none());
    }
}
