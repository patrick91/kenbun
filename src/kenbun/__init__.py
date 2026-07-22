from typing import TypedDict

from kenbun._kenbun import (
    Application,
    BuildScript,
    DeclaredDep,
    DependencySet,
    Diagnostic,
    Entrypoint,
    EnvVar,
    Evidence,
    FileEntry,
    LockfileRef,
    ManifestRef,
    NodeInfo,
    PythonInfo,
    ResolvedDep,
    ScanResult,
    SourceRef,
    Span,
    Technology,
    VersionPin,
    WantFile,
    Workspace,
    analyze,
    scan,
)


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]

__all__ = [
    "AnalysisHints",
    "Application",
    "BuildScript",
    "DeclaredDep",
    "DependencySet",
    "Diagnostic",
    "Entrypoint",
    "EnvVar",
    "Evidence",
    "FileEntry",
    "LockfileRef",
    "ManifestRef",
    "NodeInfo",
    "PythonInfo",
    "ResolvedDep",
    "ScanResult",
    "SourceRef",
    "Span",
    "Technology",
    "VersionPin",
    "WantFile",
    "Workspace",
    "analyze",
    "scan",
]
