from typing import Literal, TypeAlias, TypedDict

Ecosystem: TypeAlias = Literal["python", "node"]


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]


class FileEntry(TypedDict):
    path: str
    size: int | None


__all__ = ["AnalysisHints", "Ecosystem", "FileEntry"]
