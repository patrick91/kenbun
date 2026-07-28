//! Ecosystem-neutral application assembly.
//!
//! Python and JavaScript/TypeScript detectors contribute facts independently.
//! This module reconciles contributions that share an application directory,
//! applies cross-ecosystem rules, and constructs the public application model.

use super::node::{RawNodeDiscovery, RawNodePackage, RawTechnologySignal};
use super::python::Project;
use crate::diag;
use crate::model::*;

pub(crate) fn applications(
    projects: &[Project],
    node_discovery: &RawNodeDiscovery,
) -> Vec<Application> {
    let mut applications = python_applications(projects);
    merge_node_applications(&mut applications, node_discovery);
    applications.retain(|application| {
        application
            .technologies
            .iter()
            .any(|technology| technology.role == "primary")
    });
    applications.sort_by(|a, b| a.application_dir.cmp(&b.application_dir));
    applications
}

fn python_applications(projects: &[Project]) -> Vec<Application> {
    projects
        .iter()
        .filter(|project| project.is_python_project)
        .map(|project| {
            let mut technologies = vec![Technology {
                name: "python".to_string(),
                kind: "language".to_string(),
                role: "supporting".to_string(),
                confidence: "high".to_string(),
                evidence: project.evidence.clone(),
            }];
            let mut frameworks = project.frameworks.clone();
            frameworks.extend(
                project
                    .deploy_targets
                    .iter()
                    .map(|target| target.framework.clone()),
            );
            frameworks.sort();
            frameworks.dedup();
            for framework in &frameworks {
                let target = project
                    .deploy_targets
                    .iter()
                    .find(|target| &target.framework == framework);
                technologies.push(Technology {
                    name: framework.clone(),
                    kind: "framework".to_string(),
                    role: "primary".to_string(),
                    confidence: target
                        .map(|target| target.confidence.clone())
                        .unwrap_or_else(|| "high".to_string()),
                    evidence: target
                        .map(|target| target.evidence.clone())
                        .unwrap_or_else(|| project.evidence.clone()),
                });
            }
            technologies
                .sort_by(|a, b| (&a.role, &a.kind, &a.name).cmp(&(&b.role, &b.kind, &b.name)));

            let entrypoint = project
                .deploy_targets
                .iter()
                .find_map(|target| target.entrypoint.clone());
            let mut diagnostics = project.diagnostics.clone();
            for target in &project.deploy_targets {
                diagnostics.extend(target.diagnostics.clone());
            }
            diag::sort_and_dedup(&mut diagnostics);

            Application {
                application_dir: project.path.clone(),
                name: project.name.clone(),
                technologies,
                entrypoint,
                dependencies: project.dependencies.clone().into_iter().collect(),
                build_scripts: Vec::new(),
                env_vars: project.env_vars.clone(),
                python: Some(project.python.clone()),
                node: None,
                evidence: project.evidence.clone(),
                diagnostics,
            }
        })
        .collect()
}

fn merge_node_applications(applications: &mut Vec<Application>, discovery: &RawNodeDiscovery) {
    for package in &discovery.packages {
        let framework_signals: Vec<&RawTechnologySignal> = package
            .technologies
            .iter()
            .filter(|technology| technology.kind == "framework")
            .collect();
        let existing = applications
            .iter()
            .position(|application| application.application_dir == package.path);
        if existing.is_none() && framework_signals.is_empty() && !package.vite.standalone {
            continue;
        }

        let index = existing.unwrap_or_else(|| {
            applications.push(Application {
                application_dir: package.path.clone(),
                name: package.name.clone(),
                technologies: Vec::new(),
                entrypoint: None,
                dependencies: Vec::new(),
                build_scripts: Vec::new(),
                env_vars: Vec::new(),
                python: None,
                node: None,
                evidence: Vec::new(),
                diagnostics: Vec::new(),
            });
            applications.len() - 1
        });
        let application = &mut applications[index];
        application.node = Some(NodeInfo {
            requires_node: package.requires_node.clone(),
            version_pins: package
                .version_pins
                .iter()
                .map(|pin| VersionPin {
                    source: pin.source.clone(),
                    value: pin.value.clone(),
                })
                .collect(),
        });
        let had_primary = application
            .technologies
            .iter()
            .any(|technology| technology.role == "primary");
        if application.name.is_none()
            || (!had_primary && (!framework_signals.is_empty() || package.vite.standalone))
        {
            if let Some(name) = &package.name {
                application.name = Some(name.clone());
            }
        }
        for signal in &package.technologies {
            if signal.id == "inertia" {
                continue;
            }
            let name = match signal.id.as_str() {
                "solid-js" => "solid",
                other => other,
            };
            let kind = match signal.kind.as_str() {
                "ui" => "ui-framework",
                other => other,
            };
            let role = if signal.kind == "framework"
                || (signal.id == "vite"
                    && package.vite.standalone
                    && framework_signals.is_empty()
                    && !had_primary)
            {
                "primary"
            } else {
                "supporting"
            };
            merge_technology(
                &mut application.technologies,
                Technology {
                    name: name.to_string(),
                    kind: kind.to_string(),
                    role: role.to_string(),
                    confidence: "high".to_string(),
                    evidence: node_signal_evidence(package, signal),
                },
            );
        }

        if has_declared_dependency(application, "python", "cross-inertia")
            && package.inertia.corroborated
        {
            let mut evidence = package
                .technologies
                .iter()
                .find(|technology| technology.id == "inertia")
                .map(|technology| node_signal_evidence(package, technology))
                .unwrap_or_default();
            evidence.extend(
                application
                    .dependencies
                    .iter()
                    .filter(|dependencies| dependencies.ecosystem == "python")
                    .flat_map(|dependencies| &dependencies.declared)
                    .filter(|dependency| dependency.name == "cross-inertia")
                    .map(|dependency| Evidence {
                        kind: "dependency-declared".to_string(),
                        path: dependency.source.path.clone(),
                        span: dependency.source.span.clone(),
                        detail: format!("{} in `{}`", dependency.raw, dependency.group),
                    }),
            );
            merge_technology(
                &mut application.technologies,
                Technology {
                    name: "cross-inertia".to_string(),
                    kind: "integration".to_string(),
                    role: "supporting".to_string(),
                    confidence: "high".to_string(),
                    evidence,
                },
            );
        }

        application.dependencies.push(node_dependency_set(package));
        if let Some(command) = package.scripts.get("build") {
            application.build_scripts.push(BuildScript {
                name: "build".to_string(),
                command: command.clone(),
                package_manager: package
                    .package_manager
                    .as_ref()
                    .map(|manager| manager.name.clone()),
                package_manager_version: package
                    .package_manager
                    .as_ref()
                    .and_then(|manager| manager.version.clone()),
                argv: safe_argv(command),
                source: SourceRef {
                    path: package.manifest_path.clone(),
                    span: None,
                },
            });
        }
        if package.package_manager_candidates.len() > 1 {
            application.diagnostics.push(diag::kb308(
                &package.path,
                &package.package_manager_candidates,
            ));
        }
        application
            .technologies
            .sort_by(|a, b| (&a.role, &a.kind, &a.name).cmp(&(&b.role, &b.kind, &b.name)));
        application.dependencies.sort_by(|a, b| {
            (&a.ecosystem, &a.package_manager).cmp(&(&b.ecosystem, &b.package_manager))
        });
        application
            .build_scripts
            .sort_by(|a, b| a.name.cmp(&b.name));
        application.evidence = application
            .technologies
            .iter()
            .flat_map(|technology| technology.evidence.clone())
            .collect();
        application
            .evidence
            .sort_by(|a, b| (&a.path, &a.kind, &a.detail).cmp(&(&b.path, &b.kind, &b.detail)));
        application
            .evidence
            .dedup_by(|a, b| a.kind == b.kind && a.path == b.path && a.detail == b.detail);

        let primary: Vec<String> = application
            .technologies
            .iter()
            .filter(|technology| technology.role == "primary")
            .map(|technology| technology.name.clone())
            .collect();
        if primary.len() > 1 {
            application
                .diagnostics
                .push(diag::kb101(&application.application_dir, &primary));
        }
        diag::sort_and_dedup(&mut application.diagnostics);
    }
}

fn merge_technology(technologies: &mut Vec<Technology>, incoming: Technology) {
    if let Some(existing) = technologies
        .iter_mut()
        .find(|technology| technology.name == incoming.name && technology.kind == incoming.kind)
    {
        if incoming.role == "primary" {
            existing.role = "primary".to_string();
        }
        existing.evidence.extend(incoming.evidence);
        existing
            .evidence
            .sort_by(|a, b| (&a.path, &a.kind, &a.detail).cmp(&(&b.path, &b.kind, &b.detail)));
        existing
            .evidence
            .dedup_by(|a, b| a.kind == b.kind && a.path == b.path && a.detail == b.detail);
    } else {
        technologies.push(incoming);
    }
}

fn node_signal_evidence(package: &RawNodePackage, signal: &RawTechnologySignal) -> Vec<Evidence> {
    signal
        .evidence
        .iter()
        .map(|detail| {
            let (kind, path) = if let Some(path) = detail.strip_prefix("config:") {
                ("marker-file", path.to_string())
            } else if let Some(path) = detail.strip_prefix("marker:") {
                ("marker-file", path.to_string())
            } else if detail.starts_with("script:") {
                ("build-script", package.manifest_path.clone())
            } else if detail.contains("Dependencies:")
                || detail.starts_with("dependencies:")
                || detail.starts_with("dependency:")
            {
                ("dependency-declared", package.manifest_path.clone())
            } else if detail.contains('/') || detail.contains('.') {
                ("marker-file", detail.to_string())
            } else {
                ("marker-file", package.manifest_path.clone())
            };
            Evidence {
                kind: kind.to_string(),
                path,
                span: None,
                detail: detail.clone(),
            }
        })
        .collect()
}

fn node_dependency_set(package: &RawNodePackage) -> DependencySet {
    let mut declared = Vec::new();
    for (group, dependencies) in [
        ("dependencies", &package.dependencies),
        ("devDependencies", &package.dev_dependencies),
        ("optionalDependencies", &package.optional_dependencies),
    ] {
        for (name, specifier) in dependencies {
            declared.push(DeclaredDep {
                name: name.clone(),
                raw: format!("{name}@{specifier}"),
                specifier: specifier.clone(),
                extras: Vec::new(),
                markers: None,
                group: group.to_string(),
                source: SourceRef {
                    path: package.manifest_path.clone(),
                    span: None,
                },
            });
        }
    }
    declared.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));

    DependencySet {
        ecosystem: "node".to_string(),
        package_manager: package
            .package_manager
            .as_ref()
            .map(|manager| manager.name.clone()),
        package_manager_version: package
            .package_manager
            .as_ref()
            .and_then(|manager| manager.version.clone()),
        manifests: vec![ManifestRef {
            path: package.manifest_path.clone(),
            kind: "package-json".to_string(),
        }],
        declared,
    }
}

fn has_declared_dependency(application: &Application, ecosystem: &str, name: &str) -> bool {
    application.dependencies.iter().any(|dependencies| {
        dependencies.ecosystem == ecosystem
            && dependencies
                .declared
                .iter()
                .any(|dependency| dependency.name == name)
    })
}

fn safe_argv(command: &str) -> Option<Vec<String>> {
    if command.is_empty()
        || command
            .chars()
            .any(|character| "|&;<>()$`\\\n\r\"'".contains(character))
    {
        return None;
    }
    let argv: Vec<String> = command
        .split_whitespace()
        .map(ToString::to_string)
        .collect();
    (!argv.is_empty()).then_some(argv)
}
