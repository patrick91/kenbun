from os import PathLike

def hello() -> str: ...

class Span:
    start_line: int
    start_col: int
    end_line: int
    end_col: int

class Diagnostic:
    code: str
    severity: str
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

class Dependencies:
    package_manager: str
    manifests: list[ManifestRef]
    lockfiles: list[LockfileRef]
    declared: list[DeclaredDep]
    resolved: list[ResolvedDep]

class VersionPin:
    source: str
    value: str

class PythonInfo:
    requires_python: str | None
    version_pins: list[VersionPin]

class DeployTarget:
    framework: str
    form: str
    project_path: str
    entrypoint: Entrypoint | None
    confidence: str
    recommended: bool
    env_vars: list[EnvVar]
    evidence: list[Evidence]
    diagnostics: list[Diagnostic]

class Project:
    path: str
    name: str | None
    roles: list[str]
    frameworks: list[str]
    deploy_targets: list[DeployTarget]
    dependencies: Dependencies | None
    env_vars: list[EnvVar]
    python: PythonInfo
    evidence: list[Evidence]
    diagnostics: list[Diagnostic]

class Workspace:
    kind: str
    path: str
    virtual_root: bool
    members: list[str]

class WantFile:
    path: str
    reason: str
    priority: int
    max_bytes: int
    blob_sha: str | None

class ClassificationPrimary:
    path: str
    evidence: str

class Classification:
    python: str
    uses_fastapi: str
    primary: ClassificationPrimary | None

class InputInfo:
    mode: str
    files_seen: int
    complete: bool

class ScanResult:
    schema_version: int
    root: str
    upload_root: str
    scan_origin: str
    status: str
    want_files: list[WantFile]
    input: InputInfo
    workspace: Workspace | None
    projects: list[Project]
    deploy_targets: list[DeployTarget]
    classification: Classification
    diagnostics: list[Diagnostic]
    def to_json(self) -> str: ...

def scan(
    root: str | PathLike[str],
    *,
    target_dir: str | None = None,
    entrypoint: str | None = None,
    max_files: int | None = None,
    follow_symlinks: bool = False,
    extra_ignore_files: list[str] | None = None,
) -> ScanResult: ...
