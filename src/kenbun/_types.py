from typing import Literal, TypeAlias

from typing_extensions import NotRequired, TypedDict

Ecosystem: TypeAlias = Literal["python", "node"]


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]


class FileEntry(TypedDict):
    path: str
    size: int | None
    is_symlink: NotRequired[bool]


__all__ = ["AnalysisHints", "Ecosystem", "FileEntry"]
