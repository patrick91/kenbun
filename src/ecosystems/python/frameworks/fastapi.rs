//! FastAPI identity and entrypoint detection.

mod entrypoint;

use super::{dependency_evidence, Context, DeployTarget, DetectorResult};
use crate::diag;
use crate::fileset::FileSet;
use crate::model::Entrypoint;
use entrypoint::Resolution;

const DEPENDENCIES: &[&str] = &["fastapi", "fastapi-slim"];

pub(super) fn matches_dependency(name: &str) -> bool {
    DEPENDENCIES.contains(&name)
}

pub(super) fn detect(context: &Context<'_>) -> DetectorResult {
    let fastapi_dependencies = context.dependencies.named("fastapi");
    let slim_dependencies = context.dependencies.named("fastapi-slim");
    let dependencies = || fastapi_dependencies.iter().chain(slim_dependencies);
    let fastapi_declared = dependencies().next().is_some();
    let fastapi_installable = dependencies().any(|dependency| {
        dependency.group == "project"
            || (context.package_manager == "uv"
                && matches!(dependency.group.as_str(), "dev" | "group:dev"))
    });

    let mut result = DetectorResult {
        framework: fastapi_declared.then_some("fastapi"),
        evidence: dependencies().map(dependency_evidence).collect(),
        ..DetectorResult::default()
    };
    result.evidence.extend(context.setup_evidence(DEPENDENCIES));

    let dependency_confidence_cap = fastapi_declared && !fastapi_installable;
    if dependency_confidence_cap {
        for dependency in dependencies() {
            result.diagnostics.push(diag::kb301(
                context.display_path,
                "fastapi",
                &dependency.group,
            ));
        }
        result
            .diagnostics
            .push(diag::kb307(context.display_path, "fastapi"));
    }

    let tool_entrypoint = context
        .pyproject
        .and_then(|pyproject| pyproject.tool.as_ref())
        .and_then(|tool| tool.fastapi.as_ref())
        .and_then(|fastapi| fastapi.entrypoint.as_deref());
    let should_resolve_entrypoint = !context.fs.is_virtual()
        || fastapi_declared
        || context.entrypoint_hint.is_some()
        || tool_entrypoint.is_some();
    if fastapi_declared {
        context.fs.enable_script_hints();
    }

    let mut resolution: Option<Resolution> = None;
    if should_resolve_entrypoint {
        if let Some(hint) = context.entrypoint_hint {
            allow_entrypoint_scripts(context.fs, context.dir, hint);
            match entrypoint::validate_entrypoint(context.fs, context.dir, hint, "hint") {
                Ok(value) => resolution = Some(value),
                Err(errors) => result.diagnostics.extend(errors),
            }
        }
    }
    if should_resolve_entrypoint && resolution.is_none() {
        if let Some(specification) = tool_entrypoint {
            allow_entrypoint_scripts(context.fs, context.dir, specification);
            match entrypoint::validate_entrypoint(
                context.fs,
                context.dir,
                specification,
                "tool-fastapi",
            ) {
                Ok(value) => resolution = Some(value),
                Err(errors) => result.diagnostics.extend(errors),
            }
        }
    }
    let mut router_only = false;
    if should_resolve_entrypoint && resolution.is_none() {
        let scan = entrypoint::resolve_project(context.fs, context.dir);
        result.diagnostics.extend(scan.diagnostics);
        result.evidence.extend(scan.evidence);
        router_only = scan.router_only;
        let _ = scan.fastapi_import_seen;
        resolution = scan.resolution;
    }

    if let Some(resolution) = resolution {
        let mut confidence_caps = 0;
        if resolution.rule == 4 {
            confidence_caps += 1;
        }
        if resolution.is_factory {
            confidence_caps += 1;
        }
        if dependency_confidence_cap {
            confidence_caps += 1;
        }
        let confidence = if !fastapi_declared {
            "low"
        } else if confidence_caps == 0 {
            "high"
        } else if confidence_caps == 1 {
            "medium"
        } else {
            "low"
        };
        let source = match resolution.rule {
            1 => "hint",
            2 => "tool-fastapi",
            _ => "inferred",
        };
        result.deploy_targets.push(DeployTarget {
            framework: "fastapi".to_string(),
            entrypoint: Some(Entrypoint {
                kind: "asgi".to_string(),
                module: resolution.module.clone(),
                attribute: resolution.attribute.clone(),
                is_factory: resolution.is_factory,
                import_root: resolution.import_root.clone(),
                source: source.to_string(),
                as_string: format!("{}:{}", resolution.module, resolution.attribute),
            }),
            confidence: confidence.to_string(),
            evidence: resolution.evidence,
            diagnostics: resolution.diagnostics,
        });
    } else if fastapi_declared {
        result.deploy_targets.push(DeployTarget {
            framework: "fastapi".to_string(),
            entrypoint: None,
            confidence: if dependency_confidence_cap {
                "low"
            } else {
                "medium"
            }
            .to_string(),
            evidence: Vec::new(),
            diagnostics: vec![diag::kb103(context.display_path, "fastapi")],
        });
        if router_only {
            result.diagnostics.push(diag::kb104(context.display_path));
        }
    } else if router_only {
        result.diagnostics.push(diag::kb104(context.display_path));
    }

    result
}

fn allow_entrypoint_scripts(fs: &FileSet, project_dir: &str, specification: &str) {
    let Some((module, _)) = specification.split_once(':') else {
        return;
    };
    let module_path = module.replace('.', "/");
    for root in ["", "src"] {
        let base = match (
            project_dir.is_empty() || project_dir == ".",
            root.is_empty(),
        ) {
            (true, true) => String::new(),
            (true, false) => root.to_string(),
            (false, true) => project_dir.to_string(),
            (false, false) => format!("{project_dir}/{root}"),
        };
        let prefix = if base.is_empty() {
            String::new()
        } else {
            format!("{base}/")
        };
        fs.allow_script(format!("{prefix}{module_path}.py"));
        fs.allow_script(format!("{prefix}{module_path}/__init__.py"));
    }
}
