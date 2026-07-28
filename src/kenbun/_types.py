from typing import TypedDict


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]


class FileEntry(TypedDict):
    path: str
    size: int | None


__all__ = ["AnalysisHints", "FileEntry"]
