//! Diagnostic constructors. Codes are stable; messages are presentation.

use crate::model::{Diagnostic, Span};

pub const ERROR: &str = "error";
pub const WARNING: &str = "warning";
pub const INFO: &str = "info";

pub fn new(
    code: &str,
    severity: &str,
    message: impl Into<String>,
    path: Option<String>,
    span: Option<Span>,
) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: severity.to_string(),
        message: message.into(),
        path,
        span,
    }
}

pub fn kb100() -> Diagnostic {
    new(
        "KB100",
        INFO,
        "no recognizable application manifest found in scan root",
        None,
        None,
    )
}

pub fn kb101(path: &str, frameworks: &[String]) -> Diagnostic {
    new(
        "KB101",
        WARNING,
        format!(
            "multiple primary technologies detected in one application: {}",
            frameworks.join(", ")
        ),
        Some(path.to_string()),
        None,
    )
}

pub fn kb102() -> Diagnostic {
    new(
        "KB102",
        INFO,
        "project manifests found, but no supported application was detected",
        None,
        None,
    )
}

pub fn kb103(path: &str, framework: &str) -> Diagnostic {
    new(
        "KB103",
        ERROR,
        format!("{framework} dependency declared but no app object found"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb104(path: &str) -> Diagnostic {
    new(
        "KB104",
        INFO,
        "APIRouter usage found but no FastAPI app (router-only)",
        Some(path.to_string()),
        None,
    )
}

pub fn kb111(path: &str, entrypoint: &str) -> Diagnostic {
    new(
        "KB111",
        INFO,
        format!(
            "entrypoint {entrypoint} is resolvable statically but not discoverable by runtime \
             conventions; suggest adding `[tool.fastapi] entrypoint = \"{entrypoint}\"` to pyproject.toml"
        ),
        Some(path.to_string()),
        None,
    )
}

pub fn kb112(path: &str, attribute: &str) -> Diagnostic {
    new(
        "KB112",
        WARNING,
        format!(
            "factory-only entrypoint `{attribute}`: convention-based runners (fastapi run) call \
             instances, not factories"
        ),
        Some(path.to_string()),
        None,
    )
}

pub fn kb200(path: &str, message: &str, span: Span) -> Diagnostic {
    new(
        "KB200",
        ERROR,
        format!("Python syntax error: {message}"),
        Some(path.to_string()),
        Some(span),
    )
}

pub fn kb201(path: &str, message: &str) -> Diagnostic {
    new(
        "KB201",
        ERROR,
        format!("pyproject.toml is not valid TOML: {message}"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb202(path: &str) -> Diagnostic {
    new(
        "KB202",
        WARNING,
        "pyproject.toml has no [project] table (and is not a workspace root)",
        Some(path.to_string()),
        None,
    )
}

pub fn kb203(path: &str, message: &str) -> Diagnostic {
    new(
        "KB203",
        WARNING,
        format!("Node manifest or workspace metadata could not be parsed: {message}"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb300(path: &str) -> Diagnostic {
    new(
        "KB300",
        WARNING,
        "both pyproject.toml and requirements.txt declare dependencies at the same root",
        Some(path.to_string()),
        None,
    )
}

pub fn kb301(path: &str, framework: &str, group: &str) -> Diagnostic {
    new(
        "KB301",
        WARNING,
        format!(
            "{framework} is declared only in `{group}`, which is not on the evident install path"
        ),
        Some(path.to_string()),
        None,
    )
}

pub fn kb305(path: &str, lockfiles: &[String]) -> Diagnostic {
    new(
        "KB305",
        WARNING,
        format!("multiple lockfiles present: {}", lockfiles.join(", ")),
        Some(path.to_string()),
        None,
    )
}

pub fn kb306(path: &str, manager: &str) -> Diagnostic {
    new(
        "KB306",
        INFO,
        format!("non-uv package manager detected ({manager})"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb307(path: &str, framework: &str) -> Diagnostic {
    new(
        "KB307",
        ERROR,
        format!(
            "no installable dependency source for this application: {framework} is declared \
             only in sources standard installers do not read"
        ),
        Some(path.to_string()),
        None,
    )
}

pub fn kb308(path: &str, candidates: &[String]) -> Diagnostic {
    new(
        "KB308",
        WARNING,
        format!(
            "Node package manager is ambiguous: {}",
            candidates.join(", ")
        ),
        Some(path.to_string()),
        None,
    )
}

pub fn kb400(root: &str, matched: &str) -> Diagnostic {
    new(
        "KB400",
        ERROR,
        format!("workspace member `{matched}` is missing a pyproject.toml"),
        Some(root.to_string()),
        None,
    )
}

pub fn kb401(member: &str) -> Diagnostic {
    new(
        "KB401",
        ERROR,
        "nested workspace: member declares its own [tool.uv.workspace]",
        Some(member.to_string()),
        None,
    )
}

pub fn kb402(root: &str, pattern: &str) -> Diagnostic {
    new(
        "KB402",
        WARNING,
        format!("workspace members glob `{pattern}` matched nothing"),
        Some(root.to_string()),
        None,
    )
}

pub fn kb500(application_dir: &str) -> Diagnostic {
    new(
        "KB500",
        ERROR,
        format!("application_dir `{application_dir}` does not exist"),
        None,
        None,
    )
}

pub fn kb501(application_dir: &str) -> Diagnostic {
    new(
        "KB501",
        ERROR,
        format!("application_dir `{application_dir}` escapes the scan root"),
        None,
        None,
    )
}

pub fn kb502(application_dir: &str) -> Diagnostic {
    new(
        "KB502",
        ERROR,
        format!("application_dir `{application_dir}` is not a detected application"),
        None,
        None,
    )
}

pub fn kb503(entrypoint: &str) -> Diagnostic {
    new(
        "KB503",
        ERROR,
        format!("entrypoint module `{entrypoint}` not found under any computed import root"),
        None,
        None,
    )
}

pub fn kb504(entrypoint: &str) -> Diagnostic {
    new(
        "KB504",
        ERROR,
        format!("entrypoint attribute in `{entrypoint}` not found at module level"),
        None,
        None,
    )
}

pub fn kb505(path: &str, attribute: &str) -> Diagnostic {
    new(
        "KB505",
        WARNING,
        format!("entrypoint attribute `{attribute}` exists but does not look like an app object"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb700(path: &str, detail: &str) -> Diagnostic {
    new(
        "KB700",
        WARNING,
        format!("contradictory Python version pins: {detail}"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb802(limit: u64) -> Diagnostic {
    new(
        "KB802",
        WARNING,
        format!("scan budget exceeded (max_files={limit}); result is partial"),
        None,
        None,
    )
}

pub fn kb800(path: &str, message: &str) -> Diagnostic {
    new(
        "KB800",
        ERROR,
        format!("scan root is unavailable: {message}"),
        Some(path.to_string()),
        None,
    )
}

pub fn kb801(path: &str, message: &str) -> Diagnostic {
    new(
        "KB801",
        WARNING,
        format!("filesystem entry was omitted: {message}"),
        Some(path.to_string()),
        None,
    )
}
