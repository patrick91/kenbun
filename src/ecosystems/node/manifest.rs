//! `package.json` parsing into deterministic Node package facts.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::workspace::package_workspace_patterns;
use super::{raw_error, RawNodeParseError};

#[derive(Clone, Debug, Default)]
pub(super) struct PackageManifest {
    pub parsed: bool,
    pub name: Option<String>,
    pub dependencies: BTreeMap<String, String>,
    pub dev_dependencies: BTreeMap<String, String>,
    pub optional_dependencies: BTreeMap<String, String>,
    pub scripts: BTreeMap<String, String>,
    pub package_manager: Option<String>,
    pub requires_node: Option<String>,
    pub declares_workspace: bool,
    pub workspace_patterns: Vec<String>,
}

pub(super) fn parse_package_json(
    path: &str,
    source: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> PackageManifest {
    let value: Value = match serde_json::from_str(source) {
        Ok(value) => value,
        Err(error) => {
            errors.push(raw_error(path, format!("invalid JSON: {error}")));
            return PackageManifest::default();
        }
    };
    let Some(object) = value.as_object() else {
        errors.push(raw_error(path, "package.json root must be an object"));
        return PackageManifest::default();
    };

    let name = optional_string(object, "name", path, errors);
    let package_manager = optional_string(object, "packageManager", path, errors);
    let requires_node = string_map(object, "engines", path, errors).remove("node");
    let dependencies = string_map(object, "dependencies", path, errors);
    let dev_dependencies = string_map(object, "devDependencies", path, errors);
    let optional_dependencies = string_map(object, "optionalDependencies", path, errors);
    let scripts = string_map(object, "scripts", path, errors);
    let (declares_workspace, mut workspace_patterns) =
        package_workspace_patterns(object.get("workspaces"), path, errors);
    workspace_patterns.sort();
    workspace_patterns.dedup();

    PackageManifest {
        parsed: true,
        name,
        dependencies,
        dev_dependencies,
        optional_dependencies,
        scripts,
        package_manager,
        requires_node,
        declares_workspace,
        workspace_patterns,
    }
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> Option<String> {
    match object.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            errors.push(raw_error(path, format!("`{key}` must be a string")));
            None
        }
    }
}

fn string_map(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> BTreeMap<String, String> {
    let Some(value) = object.get(key) else {
        return BTreeMap::new();
    };
    let Some(values) = value.as_object() else {
        errors.push(raw_error(path, format!("`{key}` must be an object")));
        return BTreeMap::new();
    };
    let mut parsed = BTreeMap::new();
    for (name, value) in values {
        if let Some(value) = value.as_str() {
            parsed.insert(name.clone(), value.to_string());
        } else {
            errors.push(raw_error(path, format!("`{key}.{name}` must be a string")));
        }
    }
    parsed
}
