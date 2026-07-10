from os import PathLike
from typing import Literal

class Span:
    start_line: int
    start_col: int
    end_line: int
    end_col: int

class Diagnostic:
    code: str
    severity: Literal["error", "warning", "info"]
    message: str
    path: str | None
    span: Span | None

class Evidence:
    kind: str
    path: str
    span: Span | None
    detail: str

class Entrypoint:
    kind: str
    module: str
    attribute: str
    is_factory: bool
    import_root: str
    source: str
    as_string: str

class EnvVar:
    names: list[str]
    required: bool
    case_sensitive: bool
    value_kind: str
    has_default: bool
    default_is_computed: bool
    source: str
    origin_path: str
    origin_span: Span | None
    origin_symbol: str
    confidence: str

class SourceRef:
    path: str
    span: Span | None

class DeclaredDep:
    name: str
    raw: str
    specifier: str
    extras: list[str]
    markers: str | None
    group: str
    source: SourceRef

class ResolvedDep:
    name: str
    version: str
    source: str
    marker: str | None

class ManifestRef:
    path: str
    kind: str

class LockfileRef:
    path: str
    kind: str
    parsed: bool

class DependencySet:
    ecosystem: Literal["python", "node"]
    package_manager: str | None
    manifests: list[ManifestRef]
    lockfiles: list[LockfileRef]
    declared: list[DeclaredDep]
    resolved: list[ResolvedDep]

class Technology:
    name: str
    kind: Literal["language", "framework", "ui-framework", "integration", "build-tool"]
    role: Literal["primary", "supporting"]
    confidence: Literal["high", "medium", "low"]
    evidence: list[Evidence]

class BuildScript:
    name: str
    command: str
    package_manager: str | None
    argv: list[str] | None
    source: SourceRef

class VersionPin:
    source: str
    value: str

class PythonInfo:
    requires_python: str | None
    version_pins: list[VersionPin]

class NodeInfo:
    requires_node: str | None
    version_pins: list[VersionPin]

class Application:
    application_dir: str
    name: str | None
    technologies: list[Technology]
    entrypoint: Entrypoint | None
    dependencies: list[DependencySet]
    build_scripts: list[BuildScript]
    env_vars: list[EnvVar]
    python: PythonInfo | None
    node: NodeInfo | None
    evidence: list[Evidence]
    diagnostics: list[Diagnostic]

class Workspace:
    kind: Literal["uv", "npm", "pnpm", "yarn", "bun", "node", "mixed"]
    path: str
    virtual_root: bool
    members: list[str]

class ScanResult:
    schema_version: int
    root: str
    upload_root: str
    scan_origin: str
    workspace: Workspace | None
    applications: list[Application]
    diagnostics: list[Diagnostic]
    def to_json(self) -> str: ...

def scan(
    root: str | PathLike[str],
    *,
    application_dir: str | None = None,
    entrypoint: str | None = None,
    max_files: int | None = None,
    follow_symlinks: bool = False,
    extra_ignore_files: list[str] | None = None,
) -> ScanResult: ...
