//! Declarative Python and Node runtime-version facts.

use crate::fileset::FileSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimePin {
    pub source: String,
    pub value: String,
}

pub(crate) fn python_version_pins(fs: &FileSet, dir: &str) -> Vec<RuntimePin> {
    let mut pins = nearest_plain_file(fs, dir, ".python-version");
    pins.extend(nearest_tool_versions(fs, dir, &["python"]));
    sort_pins(&mut pins);
    pins
}

pub(crate) fn node_version_pins(fs: &FileSet, dir: &str) -> Vec<RuntimePin> {
    let mut pins = nearest_plain_file(fs, dir, ".node-version");
    pins.extend(nearest_plain_file(fs, dir, ".nvmrc"));
    pins.extend(nearest_tool_versions(fs, dir, &["node", "nodejs"]));
    sort_pins(&mut pins);
    pins
}

fn nearest_plain_file(fs: &FileSet, dir: &str, name: &str) -> Vec<RuntimePin> {
    for ancestor in ancestors_inclusive(dir) {
        let path = join(&ancestor, name);
        if !fs.contains(&path) {
            continue;
        }
        let Some(source) = fs.read_str(&path) else {
            return Vec::new();
        };
        return version_values(&source)
            .into_iter()
            .map(|value| RuntimePin {
                source: path.clone(),
                value,
            })
            .collect();
    }
    Vec::new()
}

fn nearest_tool_versions(fs: &FileSet, dir: &str, tools: &[&str]) -> Vec<RuntimePin> {
    for ancestor in ancestors_inclusive(dir) {
        let path = join(&ancestor, ".tool-versions");
        if !fs.contains(&path) {
            continue;
        }
        let Some(source) = fs.read_str(&path) else {
            return Vec::new();
        };
        for line in source.lines() {
            let line = line.split('#').next().unwrap_or(line).trim();
            let mut parts = line.split_whitespace();
            let Some(tool) = parts.next() else {
                continue;
            };
            if !tools.contains(&tool) {
                continue;
            }
            return parts
                .filter(|value| !value.is_empty())
                .map(|value| RuntimePin {
                    source: path.clone(),
                    value: value.to_string(),
                })
                .collect();
        }
    }
    Vec::new()
}

fn version_values(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| line.split('#').next())
        .flat_map(str::split_whitespace)
        .map(str::to_string)
        .collect()
}

fn ancestors_inclusive(dir: &str) -> Vec<String> {
    let mut ancestors = Vec::new();
    let mut current = if dir == "." { "" } else { dir };
    loop {
        ancestors.push(current.to_string());
        let Some((parent, _)) = current.rsplit_once('/') else {
            if !current.is_empty() {
                ancestors.push(String::new());
            }
            break;
        };
        current = parent;
    }
    ancestors
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() || dir == "." {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn sort_pins(pins: &mut Vec<RuntimePin>) {
    pins.sort_by(|a, b| (&a.source, &a.value).cmp(&(&b.source, &b.value)));
    pins.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestors_reach_the_fileset_root() {
        assert_eq!(
            ancestors_inclusive("apps/api/src"),
            ["apps/api/src", "apps/api", "apps", ""]
        );
        assert_eq!(ancestors_inclusive(""), [""]);
    }

    #[test]
    fn plain_version_files_support_comments_and_multiple_values() {
        assert_eq!(
            version_values("3.13.1 # primary\n3.12.8\n"),
            ["3.13.1", "3.12.8"]
        );
    }
}
