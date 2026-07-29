//! Node workspace declaration parsing, glob normalization, and expansion.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{display_dir, raw_error, RawNodeParseError};

pub(super) fn package_workspace_patterns(
    value: Option<&Value>,
    path: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> (bool, Vec<String>) {
    let Some(value) = value else {
        return (false, Vec::new());
    };
    let packages = match value {
        Value::Array(packages) => packages,
        Value::Object(object) => {
            let Some(packages) = object.get("packages") else {
                errors.push(raw_error(
                    path,
                    "`workspaces` object must contain a `packages` array",
                ));
                return (true, Vec::new());
            };
            let Some(packages) = packages.as_array() else {
                errors.push(raw_error(path, "`workspaces.packages` must be an array"));
                return (true, Vec::new());
            };
            packages
        }
        _ => {
            errors.push(raw_error(
                path,
                "`workspaces` must be an array or an object with `packages`",
            ));
            return (true, Vec::new());
        }
    };

    let mut patterns = Vec::new();
    for (index, value) in packages.iter().enumerate() {
        if let Some(pattern) = value.as_str() {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                errors.push(raw_error(
                    path,
                    format!("`workspaces` pattern at index {index} is empty"),
                ));
            } else {
                patterns.push(pattern.to_string());
            }
        } else {
            errors.push(raw_error(
                path,
                format!("`workspaces` pattern at index {index} must be a string"),
            ));
        }
    }
    (true, patterns)
}

/// Parse only the root `packages` sequence used by pnpm. Unsupported YAML
/// constructs become errors rather than being guessed or evaluated.
pub(crate) fn parse_pnpm_workspace_yaml(source: &str) -> (Vec<String>, Vec<String>) {
    let mut patterns = Vec::new();
    let mut errors = Vec::new();
    let mut in_packages = false;
    let mut saw_packages = false;

    for (index, original) in source.lines().enumerate() {
        let line_number = index + 1;
        let without_comment = strip_yaml_comment(original);
        let trimmed = without_comment.trim();
        if trimmed.is_empty() || trimmed == "---" || trimmed == "..." {
            continue;
        }
        let indentation = without_comment.len() - without_comment.trim_start().len();

        if indentation == 0 {
            if in_packages && (trimmed == "-" || trimmed.starts_with("- ")) {
                let item = trimmed.strip_prefix('-').unwrap_or(trimmed).trim();
                match parse_yaml_scalar(item) {
                    Ok(pattern) => patterns.push(pattern),
                    Err(message) => errors.push(format!("line {line_number}: {message}")),
                }
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("packages:") {
                if saw_packages {
                    errors.push(format!("duplicate `packages` key at line {line_number}"));
                }
                saw_packages = true;
                in_packages = true;
                let rest = rest.trim();
                if rest.is_empty() {
                    continue;
                }
                if rest == "[]" {
                    in_packages = false;
                    continue;
                }
                match parse_inline_yaml_list(rest) {
                    Ok(items) => patterns.extend(items),
                    Err(message) => errors.push(format!("line {line_number}: {message}")),
                }
                in_packages = false;
                continue;
            }
            in_packages = false;
            continue;
        }

        if in_packages {
            let Some(item) = trimmed.strip_prefix('-') else {
                errors.push(format!(
                    "line {line_number}: expected a `- <workspace glob>` list item"
                ));
                continue;
            };
            match parse_yaml_scalar(item.trim()) {
                Ok(pattern) => patterns.push(pattern),
                Err(message) => errors.push(format!("line {line_number}: {message}")),
            }
        }
    }

    if !saw_packages {
        errors.push("missing root `packages` key".to_string());
    }
    patterns.sort();
    patterns.dedup();
    (patterns, errors)
}

fn strip_yaml_comment(line: &str) -> String {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if double => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '#' if !single && !double => return line[..index].to_string(),
            _ => {}
        }
    }
    line.to_string()
}

fn parse_inline_yaml_list(value: &str) -> Result<Vec<String>, String> {
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return Err("only a YAML list is supported after `packages:`".to_string());
    };
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if double => {
                current.push(character);
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                current.push(character);
            }
            '"' if !single => {
                double = !double;
                current.push(character);
            }
            ',' if !single && !double => {
                items.push(parse_yaml_scalar(current.trim())?);
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if single || double {
        return Err("unterminated quoted scalar".to_string());
    }
    items.push(parse_yaml_scalar(current.trim())?);
    Ok(items)
}

fn parse_yaml_scalar(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches(',').trim();
    if value.is_empty() {
        return Err("workspace glob is empty".to_string());
    }
    if value.starts_with('\'') {
        let Some(inner) = value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
        else {
            return Err("unterminated single-quoted workspace glob".to_string());
        };
        let parsed = inner.replace("''", "'");
        if parsed.is_empty() {
            return Err("workspace glob is empty".to_string());
        }
        return Ok(parsed);
    }
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map_err(|error| format!("invalid double-quoted workspace glob: {error}"))
            .and_then(|parsed| {
                if parsed.is_empty() {
                    Err("workspace glob is empty".to_string())
                } else {
                    Ok(parsed)
                }
            });
    }
    if value.starts_with('[') || value.starts_with('{') || value.contains('\n') {
        return Err("unsupported YAML workspace glob construct".to_string());
    }
    Ok(value.to_string())
}

pub(super) fn expand_workspace_patterns(
    patterns: &[String],
    package_dirs: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut included = BTreeSet::new();
    let mut excluded = BTreeSet::new();
    let mut unmatched = Vec::new();
    let mut errors = Vec::new();

    for original in patterns {
        let (negative, raw) = original
            .strip_prefix('!')
            .map_or((false, original.as_str()), |value| (true, value));
        let normalized = match normalize_workspace_pattern(raw) {
            Ok(pattern) => pattern,
            Err(message) => {
                errors.push(format!("invalid workspace glob `{original}`: {message}"));
                continue;
            }
        };
        let options = glob::MatchOptions {
            require_literal_separator: true,
            ..glob::MatchOptions::default()
        };
        let mut matches = BTreeSet::new();
        for expanded in expand_braces(&normalized) {
            let pattern = match glob::Pattern::new(&expanded) {
                Ok(pattern) => pattern,
                Err(error) => {
                    errors.push(format!("invalid workspace glob `{original}`: {error}"));
                    continue;
                }
            };
            matches.extend(
                package_dirs
                    .iter()
                    .filter(|directory| pattern.matches_with(&display_dir(directory), options))
                    .cloned(),
            );
        }
        if matches.is_empty() && !negative {
            unmatched.push(original.clone());
        }
        if negative {
            excluded.extend(matches);
        } else {
            included.extend(matches);
        }
    }

    for directory in excluded {
        included.remove(&directory);
    }
    let members = included
        .into_iter()
        .map(|directory| display_dir(&directory))
        .collect();
    unmatched.sort();
    unmatched.dedup();
    errors.sort();
    errors.dedup();
    (members, unmatched, errors)
}

fn normalize_workspace_pattern(pattern: &str) -> Result<String, String> {
    let mut pattern = pattern.trim().replace('\\', "/");
    while let Some(stripped) = pattern.strip_prefix("./") {
        pattern = stripped.to_string();
    }
    pattern = pattern
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string();
    if let Some(stripped) = pattern.strip_suffix("/package.json") {
        pattern = stripped.to_string();
    }
    if pattern.is_empty() {
        return Err("empty pattern".to_string());
    }
    if pattern.split('/').any(|part| part == "..") {
        return Err("pattern escapes the workspace root".to_string());
    }
    Ok(pattern)
}

pub(crate) fn workspace_pattern_matches(pattern: &str, relative_path: &str) -> bool {
    let Ok(normalized) = normalize_workspace_pattern(pattern) else {
        return false;
    };
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::default()
    };
    expand_braces(&normalized).into_iter().any(|expanded| {
        glob::Pattern::new(&expanded)
            .is_ok_and(|pattern| pattern.matches_with(relative_path, options))
    })
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(relative_close) = pattern[open + 1..].find('}') else {
        return vec![pattern.to_string()];
    };
    let close = open + 1 + relative_close;
    let alternatives: Vec<&str> = pattern[open + 1..close].split(',').collect();
    if alternatives.len() < 2 {
        return vec![pattern.to_string()];
    }
    let mut expanded = Vec::new();
    for alternative in alternatives {
        let next = format!(
            "{}{}{}",
            &pattern[..open],
            alternative,
            &pattern[close + 1..]
        );
        expanded.extend(expand_braces(&next));
    }
    expanded
}
