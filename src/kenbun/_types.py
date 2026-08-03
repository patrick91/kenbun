from typing import Literal, TypeAlias, TypedDict

Ecosystem: TypeAlias = Literal["python", "node"]


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]


class _RequiredFileEntry(TypedDict):
    path: str
    size: int | None


class FileEntry(_RequiredFileEntry, total=False):
    is_symlink: bool


__all__ = ["AnalysisHints", "Ecosystem", "FileEntry"]
