from typing import TypedDict


class AnalysisHints(TypedDict, total=False):
    script_patterns: list[str]


__all__ = ["AnalysisHints"]
