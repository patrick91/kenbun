//! Layer 2 — static entrypoint resolution (spec §6.3).
//!
//! Mirrors fastapi-cli's runtime discovery exactly, because production
//! (`fastapi run`) IS fastapi-cli: same candidate file order, same
//! first-existing-file semantics, same app > api > alphabetical precedence —
//! but resolved from the AST instead of importing user code.

use std::collections::BTreeSet;

use ruff_python_ast::{Expr, ModModule, Stmt};
use ruff_python_parser::parse_unchecked_source;
use ruff_source_file::LineIndex;

use crate::diag;
use crate::fileset::FileSet;
use crate::model::{Diagnostic, Evidence, Span};

/// fastapi-cli's exact search order (discover.py get_default_path).
pub const RULE3_CANDIDATES: &[&str] = &[
    "main.py",
    "app.py",
    "api.py",
    "app/main.py",
    "app/app.py",
    "app/api.py",
];

pub struct ModuleAnalysis {
    pub syntax_errors: Vec<(String, Span)>,
    /// names bound at module level to `FastAPI(...)` (or to a factory call)
    pub instance_bindings: Vec<String>,
    /// function names statically returning FastAPI
    pub factories: Vec<String>,
    /// (exported name, source module suffix, source attr) from `from .X import a [as b]`
    pub reexports: Vec<(String, String, String)>,
    pub router_usage: bool,
    pub fastapi_import: bool,
    /// any module-level name bound by assignment or import (for KB504/KB505)
    pub module_level_names: BTreeSet<String>,
}

fn expr_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(n) => Some(n.id.as_str()),
        _ => None,
    }
}

/// Is `expr` a call target referring to symbol `sym` given the module's
/// import table? (`FastAPI` / aliased / `fastapi.FastAPI`)
fn is_symbol_ref(
    expr: &Expr,
    direct: &BTreeSet<String>,
    module_aliases: &BTreeSet<String>,
    attr_name: &str,
) -> bool {
    match expr {
        Expr::Name(n) => direct.contains(n.id.as_str()),
        Expr::Attribute(a) => {
            a.attr.as_str() == attr_name
                && expr_name(&a.value).is_some_and(|base| module_aliases.contains(base))
        }
        _ => false,
    }
}

pub fn analyze_module(source: &str) -> ModuleAnalysis {
    let parsed = parse_unchecked_source(source, ruff_python_ast::PySourceType::Python);
    let line_index = LineIndex::from_source_text(source);

    let to_span = |range: ruff_text_size::TextRange| {
        let start = line_index.line_column(range.start(), source);
        let end = line_index.line_column(range.end(), source);
        Span {
            start_line: start.line.get() as u32,
            start_col: start.column.get() as u32,
            end_line: end.line.get() as u32,
            end_col: end.column.get() as u32,
        }
    };

    let syntax_errors: Vec<(String, Span)> = parsed
        .errors()
        .iter()
        .take(5)
        .map(|e| (e.error.to_string(), to_span(e.location)))
        .collect();

    let module: &ModModule = parsed.syntax();

    // Pass A — imports.
    let mut fastapi_symbols = BTreeSet::new(); // names referring to the FastAPI class
    let mut router_symbols = BTreeSet::new();
    let mut module_aliases = BTreeSet::new(); // names referring to the fastapi module
    let mut fastapi_import = false;
    let mut module_level_names = BTreeSet::new();
    let mut reexports = Vec::new();

    for stmt in &module.body {
        match stmt {
            Stmt::ImportFrom(imp) => {
                let module_name = imp.module.as_ref().map(|m| m.as_str()).unwrap_or("");
                for alias in &imp.names {
                    let bound = alias
                        .asname
                        .as_ref()
                        .map(|a| a.as_str())
                        .unwrap_or(alias.name.as_str());
                    module_level_names.insert(bound.to_string());
                    if module_name == "fastapi" && imp.level == 0 {
                        fastapi_import = true;
                        match alias.name.as_str() {
                            "FastAPI" => {
                                fastapi_symbols.insert(bound.to_string());
                            }
                            "APIRouter" => {
                                router_symbols.insert(bound.to_string());
                            }
                            _ => {}
                        }
                    }
                    // one-hop re-export bookkeeping: `from .X import a [as b]`
                    if imp.level == 1 && alias.name.as_str() != "*" {
                        reexports.push((
                            bound.to_string(),
                            module_name.to_string(),
                            alias.name.to_string(),
                        ));
                    }
                }
            }
            Stmt::Import(imp) => {
                for alias in &imp.names {
                    let bound = alias
                        .asname
                        .as_ref()
                        .map(|a| a.as_str())
                        .unwrap_or(alias.name.as_str());
                    module_level_names.insert(bound.to_string());
                    if alias.name.as_str() == "fastapi" {
                        fastapi_import = true;
                        module_aliases.insert(bound.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // Pass B — factories: return annotation is FastAPI, or a return statement
    // directly returns `FastAPI(...)` (spec §6.3).
    let mut factories = Vec::new();
    for stmt in &module.body {
        if let Stmt::FunctionDef(func) = stmt {
            let by_annotation = func
                .returns
                .as_ref()
                .is_some_and(|r| is_symbol_ref(r, &fastapi_symbols, &module_aliases, "FastAPI"));
            let by_return = func.body.iter().any(|s| {
                if let Stmt::Return(ret) = s {
                    if let Some(Expr::Call(call)) = ret.value.as_deref() {
                        return is_symbol_ref(
                            &call.func,
                            &fastapi_symbols,
                            &module_aliases,
                            "FastAPI",
                        );
                    }
                }
                false
            });
            if by_annotation || by_return {
                factories.push(func.name.to_string());
            }
            module_level_names.insert(func.name.to_string());
        }
    }

    // Pass C — module-level bindings.
    let mut instance_bindings = Vec::new();
    let mut router_usage = !router_symbols.is_empty();
    let factory_set: BTreeSet<&str> = factories.iter().map(String::as_str).collect();

    let mut record_binding = |target: &Expr, value: &Expr| {
        let Some(name) = expr_name(target) else {
            return;
        };
        if let Expr::Call(call) = value {
            if is_symbol_ref(&call.func, &fastapi_symbols, &module_aliases, "FastAPI") {
                instance_bindings.push(name.to_string());
            } else if is_symbol_ref(&call.func, &router_symbols, &module_aliases, "APIRouter") {
                router_usage = true;
            } else if expr_name(&call.func).is_some_and(|f| factory_set.contains(f)) {
                // `app = create_app()` — the dominant factory idiom counts as
                // an instance binding, is_factory=false (spec §6.3)
                instance_bindings.push(name.to_string());
            }
        }
    };

    for stmt in &module.body {
        match stmt {
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    if expr_name(target).is_some() {
                        module_level_names.insert(expr_name(target).unwrap().to_string());
                    }
                    record_binding(target, &assign.value);
                }
            }
            Stmt::AnnAssign(assign) => {
                if let Some(name) = expr_name(&assign.target) {
                    module_level_names.insert(name.to_string());
                }
                if let Some(value) = &assign.value {
                    record_binding(&assign.target, value);
                }
            }
            _ => {}
        }
    }

    ModuleAnalysis {
        syntax_errors,
        instance_bindings,
        factories,
        reexports,
        router_usage,
        fastapi_import,
        module_level_names,
    }
}

/// fastapi-cli variable precedence: app, api, then alphabetical (dir() order).
pub fn pick_attribute(bindings: &[String]) -> Option<String> {
    if bindings.iter().any(|b| b == "app") {
        return Some("app".into());
    }
    if bindings.iter().any(|b| b == "api") {
        return Some("api".into());
    }
    let mut sorted: Vec<&String> = bindings.iter().collect();
    sorted.sort();
    sorted.first().map(|s| s.to_string())
}

/// Dotted module path via the __init__.py walk-up (fastapi-cli semantics).
/// `file` is relative to `project_dir`; returns (module, import_root).
pub fn module_path(fs: &FileSet, project_dir: &str, file: &str) -> (String, String) {
    let join = |dir: &str, name: &str| -> String {
        if dir.is_empty() {
            name.to_string()
        } else {
            format!("{dir}/{name}")
        }
    };
    let stem = file.strip_suffix(".py").unwrap_or(file);
    let mut parts: Vec<&str> = stem.split('/').collect();
    if parts.last() == Some(&"__init__") {
        parts.pop();
    }
    // Walk up: a parent directory joins the module path iff it is a package.
    let mut module_parts = vec![parts.pop().unwrap_or_default()];
    while let Some(parent) = parts.last() {
        let parent_dir = parts.join("/");
        let init = join(project_dir, &join(&parent_dir, "__init__.py"));
        if fs.contains(&init) {
            module_parts.insert(0, parent);
            parts.pop();
        } else {
            break;
        }
    }
    let import_root = if parts.is_empty() {
        if project_dir.is_empty() {
            ".".to_string()
        } else {
            project_dir.to_string()
        }
    } else {
        join(project_dir, &parts.join("/"))
    };
    (module_parts.join("."), import_root)
}

pub struct Resolution {
    #[allow(dead_code)] // M2: KB302/anchoring diagnostics to the file
    pub file: String,
    pub module: String,
    pub attribute: String,
    pub is_factory: bool,
    pub import_root: String,
    /// 3 = fastapi-cli conventional (production will find it); 4 = extended
    pub rule: u8,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct ProjectScan {
    pub resolution: Option<Resolution>,
    pub diagnostics: Vec<Diagnostic>,
    pub evidence: Vec<Evidence>,
    pub router_only: bool,
    pub fastapi_import_seen: bool,
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() || dir == "." {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Resolve a project's entrypoint via rules 3 and 4 (§6.3). Rule 3 mirrors
/// production exactly: the FIRST EXISTING candidate file decides; if it has
/// no app object, production fails there, so later files only qualify under
/// rule 4 (medium + KB111).
pub fn resolve_project(fs: &FileSet, project_dir: &str) -> ProjectScan {
    let mut diagnostics = Vec::new();
    let mut evidence = Vec::new();
    let mut router_seen = false;
    let mut fastapi_import_seen = false;
    let mut rule3_blocked = false;

    let analyze = |rel_file: &str,
                   diagnostics: &mut Vec<Diagnostic>,
                   router: &mut bool,
                   imported: &mut bool|
     -> Option<ModuleAnalysis> {
        let source = fs.read_str(rel_file)?;
        let analysis = analyze_module(&source);
        for (msg, span) in &analysis.syntax_errors {
            diagnostics.push(diag::kb200(rel_file, msg, span.clone()));
        }
        *router |= analysis.router_usage;
        *imported |= analysis.fastapi_import;
        Some(analysis)
    };

    let make_resolution = |rel_file: &str,
                           in_project: &str,
                           analysis: &ModuleAnalysis,
                           rule: u8|
     -> Option<Resolution> {
        let mut is_factory = false;
        let mut attribute = pick_attribute(&analysis.instance_bindings);
        let mut source_note = String::new();

        // One-hop re-export: __init__.py exporting an app from a sibling (§6.3).
        if attribute.is_none() && rel_file.ends_with("__init__.py") {
            for (exported, source_module, source_attr) in &analysis.reexports {
                let pkg_dir = in_project.strip_suffix("/__init__.py").unwrap_or("");
                let source_file = join(
                    &join(project_dir, pkg_dir),
                    &format!("{}.py", source_module.replace('.', "/")),
                );
                if let Some(src) = fs.read_str(&source_file) {
                    let inner = analyze_module(&src);
                    if inner.instance_bindings.iter().any(|b| b == source_attr) {
                        attribute = Some(exported.clone());
                        source_note = format!("re-exported from {source_file}");
                        break;
                    }
                }
            }
        }

        if attribute.is_none() {
            // Factory-only (§6.3): target exists, capped medium, KB112.
            if let Some(factory) = analysis.factories.first() {
                attribute = Some(factory.clone());
                is_factory = true;
            }
        }

        let attribute = attribute?;
        let (module, import_root) = module_path(fs, project_dir, in_project);
        let mut ev = vec![Evidence {
            kind: if is_factory {
                "factory-function"
            } else {
                "app-instance"
            }
            .to_string(),
            path: rel_file.to_string(),
            span: None,
            detail: if source_note.is_empty() {
                format!("`{attribute}` bound at module level")
            } else {
                source_note
            },
        }];
        let mut diags = Vec::new();
        let as_string = format!("{module}:{attribute}");
        if is_factory {
            diags.push(diag::kb112(rel_file, &attribute));
        }
        if rule == 4 {
            diags.push(diag::kb111(rel_file, &as_string));
        }
        for other in &analysis.instance_bindings {
            if *other != attribute {
                ev.push(Evidence {
                    kind: "runner-up-candidate".to_string(),
                    path: rel_file.to_string(),
                    span: None,
                    detail: format!("`{other}` also bound to a FastAPI instance"),
                });
            }
        }
        Some(Resolution {
            file: rel_file.to_string(),
            module,
            attribute,
            is_factory,
            import_root,
            rule,
            evidence: ev,
            diagnostics: diags,
        })
    };

    // Rule 3: first existing conventional file decides, like production.
    for candidate in RULE3_CANDIDATES {
        let rel = join(project_dir, candidate);
        if !fs.contains(&rel) {
            continue;
        }
        let Some(analysis) = analyze(
            &rel,
            &mut diagnostics,
            &mut router_seen,
            &mut fastapi_import_seen,
        ) else {
            continue;
        };
        if let Some(resolution) = make_resolution(&rel, candidate, &analysis, 3) {
            return ProjectScan {
                resolution: Some(resolution),
                diagnostics,
                evidence,
                router_only: false,
                fastapi_import_seen,
            };
        }
        // Production stops at this file and fails; later hits are rule 4.
        evidence.push(Evidence {
            kind: "filename-convention".to_string(),
            path: rel.clone(),
            span: None,
            detail: "conventional entry file exists but binds no app object; production \
                     discovery would stop here"
                .to_string(),
        });
        rule3_blocked = true;
        break;
    }

    // Rule 4: remaining conventional files (when rule 3 was blocked) plus
    // package dirs under the project root and src/, in byte order.
    let mut rule4_candidates: Vec<String> = Vec::new();
    if rule3_blocked {
        for candidate in RULE3_CANDIDATES {
            rule4_candidates.push(candidate.to_string());
        }
    }
    for base in ["", "src"] {
        let base_dir = join(project_dir, base);
        let base_dir = if base_dir == "." {
            String::new()
        } else {
            base_dir
        };
        let mut pkgs = BTreeSet::new();
        let prefix_len = if base_dir.is_empty() {
            0
        } else {
            base_dir.len() + 1
        };
        for file in fs.under(if base_dir.is_empty() {
            project_dir
        } else {
            &base_dir
        }) {
            let local = &file[if base_dir.is_empty() {
                if project_dir.is_empty() {
                    0
                } else {
                    project_dir.len() + 1
                }
            } else {
                prefix_len
            }..];
            if let Some((pkg, rest)) = local.split_once('/') {
                if rest == "__init__.py" {
                    pkgs.insert(pkg.to_string());
                }
            }
        }
        for pkg in pkgs {
            let pkg_rel = if base.is_empty() {
                pkg.clone()
            } else {
                format!("{base}/{pkg}")
            };
            for name in ["main.py", "app.py", "api.py", "__init__.py"] {
                rule4_candidates.push(format!("{pkg_rel}/{name}"));
            }
        }
    }

    let mut seen = BTreeSet::new();
    for candidate in rule4_candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        let rel = join(project_dir, &candidate);
        if !fs.contains(&rel) {
            continue;
        }
        let Some(analysis) = analyze(
            &rel,
            &mut diagnostics,
            &mut router_seen,
            &mut fastapi_import_seen,
        ) else {
            continue;
        };
        if let Some(resolution) = make_resolution(&rel, &candidate, &analysis, 4) {
            return ProjectScan {
                resolution: Some(resolution),
                diagnostics,
                evidence,
                router_only: false,
                fastapi_import_seen,
            };
        }
    }

    ProjectScan {
        resolution: None,
        diagnostics,
        evidence,
        router_only: router_seen,
        fastapi_import_seen,
    }
}

/// Validate an explicit `module:attr` (rule 1 hint / rule 2 tool.fastapi).
/// Returns Ok(resolution) or Err(diagnostics) — spec §10.
pub fn validate_entrypoint(
    fs: &FileSet,
    project_dir: &str,
    spec: &str,
    source: &str,
) -> Result<Resolution, Vec<Diagnostic>> {
    let Some((module, attribute)) = spec.split_once(':') else {
        return Err(vec![diag::kb503(spec)]);
    };
    let module_rel = module.replace('.', "/");
    let mut found = None;
    for root in ["", "src"] {
        let base = join(project_dir, root);
        let base = if base == "." { String::new() } else { base };
        for suffix in [
            format!("{module_rel}.py"),
            format!("{module_rel}/__init__.py"),
        ] {
            let rel = join(&base, &suffix);
            if fs.contains(&rel) {
                found = Some((
                    rel,
                    if base.is_empty() {
                        ".".to_string()
                    } else {
                        base.clone()
                    },
                ));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let Some((file, import_root)) = found else {
        return Err(vec![diag::kb503(spec)]);
    };
    let Some(src) = fs.read_str(&file) else {
        return Err(vec![diag::kb503(spec)]);
    };
    let analysis = analyze_module(&src);
    let mut diags: Vec<Diagnostic> = analysis
        .syntax_errors
        .iter()
        .map(|(msg, span)| diag::kb200(&file, msg, span.clone()))
        .collect();

    if !analysis.module_level_names.contains(attribute) {
        diags.push(diag::kb504(spec));
        return Err(diags);
    }
    let is_instance = analysis.instance_bindings.iter().any(|b| b == attribute);
    let is_factory = analysis.factories.iter().any(|f| f == attribute);
    if !is_instance && !is_factory {
        diags.push(diag::new(
            "KB505",
            diag::WARNING,
            format!(
                "entrypoint attribute `{attribute}` exists but does not look like an app object"
            ),
            Some(file.clone()),
            None,
        ));
    }
    Ok(Resolution {
        file: file.clone(),
        module: module.to_string(),
        attribute: attribute.to_string(),
        is_factory: is_factory && !is_instance,
        import_root,
        rule: if source == "hint" { 1 } else { 2 },
        evidence: vec![Evidence {
            kind: "config-entrypoint".to_string(),
            path: file,
            span: None,
            detail: format!("validated {source} entrypoint `{spec}`"),
        }],
        diagnostics: diags,
    })
}
