from typing import TypedDict


class _RequiredFileEntry(TypedDict):
    path: str
    size: int


class FileEntry(_RequiredFileEntry, total=False):
    blob_sha: str | None


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]


__all__ = ["AnalysisHints", "FileEntry"]
