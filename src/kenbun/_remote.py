from __future__ import annotations

from collections.abc import Iterable, Mapping

from kenbun._kenbun import FileRequest, ScanResult, analyze
from kenbun._types import AnalysisHints, Ecosystem, FileEntry

DEFAULT_MAX_ROUNDS = 20
DEFAULT_MAX_FILE_BYTES = 2 * 1024 * 1024


class RemoteAnalysis:
    """A stateful remote repository analysis."""

    def __init__(
        self,
        files: Iterable[FileEntry],
        *,
        ecosystems: Iterable[Ecosystem] | None = None,
        inventory_complete: bool = True,
        hints: AnalysisHints | None = None,
        max_rounds: int = DEFAULT_MAX_ROUNDS,
        max_files: int | None = None,
        max_file_bytes: int = DEFAULT_MAX_FILE_BYTES,
    ) -> None:
        if (
            not isinstance(max_rounds, int)
            or isinstance(max_rounds, bool)
            or max_rounds < 1
        ):
            raise ValueError("max_rounds must be a positive integer")
        if max_files is not None and (
            not isinstance(max_files, int)
            or isinstance(max_files, bool)
            or max_files < 0
        ):
            raise ValueError("max_files must be a non-negative integer or None")
        if (
            not isinstance(max_file_bytes, int)
            or isinstance(max_file_bytes, bool)
            or max_file_bytes < 1
        ):
            raise ValueError("max_file_bytes must be a positive integer")
        if isinstance(ecosystems, str):
            raise TypeError(
                "ecosystems must be an iterable of ecosystem names, not a string"
            )

        self._files = tuple(files)
        self._contents: dict[str, bytes | None] = {}
        self._ecosystems = None if ecosystems is None else tuple(ecosystems)
        self._inventory_complete = inventory_complete
        self._hints = hints
        self._max_rounds = max_rounds
        self._max_files = max_files
        self._max_file_bytes = max_file_bytes
        self._current = self._analyze()
        self._validate_current()
        self._round_number = 1 if self.file_requests else 0

    @property
    def file_requests(self) -> list[FileRequest]:
        return self._current.file_requests

    @property
    def round_number(self) -> int:
        return self._round_number

    @property
    def result(self) -> ScanResult:
        if self.file_requests:
            raise RuntimeError("Remote analysis is not complete")
        return self._current

    def update(self, contents: Mapping[str, bytes | None]) -> None:
        if not self.file_requests:
            raise RuntimeError("Remote analysis is already complete")

        response = dict(contents)
        requested_paths = {request.path for request in self.file_requests}
        missing = requested_paths - response.keys()
        if missing:
            paths = ", ".join(sorted(missing))
            raise ValueError(f"Missing responses for requested files: {paths}")
        unexpected = response.keys() - requested_paths
        if unexpected:
            paths = ", ".join(sorted(unexpected))
            raise ValueError(f"Received unrequested files: {paths}")

        for path, content in response.items():
            if content is not None and not isinstance(content, bytes):
                raise TypeError(f"Content for {path!r} must be bytes or None")
            if content is not None and len(content) > self._max_file_bytes:
                raise ValueError(
                    f"Content for {path!r} exceeds its "
                    f"{self._max_file_bytes}-byte limit"
                )

        self._contents.update(response)
        self._current = self._analyze()
        self._validate_current()

        resolved_paths = self._contents.keys()
        repeated = [
            request.path
            for request in self._current.file_requests
            if request.path in resolved_paths
        ]
        if repeated:
            paths = ", ".join(repeated)
            raise RuntimeError(f"Kenbun requested already resolved files: {paths}")

        if self.file_requests:
            self._round_number += 1
            if self._round_number > self._max_rounds:
                raise RuntimeError(
                    f"Remote analysis did not finish within {self._max_rounds} rounds"
                )

    def _analyze(self) -> ScanResult:
        return analyze(
            self._files,
            self._contents,
            ecosystems=self._ecosystems,
            inventory_complete=self._inventory_complete,
            hints=self._hints,
            max_files=self._max_files,
            max_file_bytes=self._max_file_bytes,
        )

    def _validate_current(self) -> None:
        if self._current.status == "needs_files" and not self.file_requests:
            raise RuntimeError(
                "Kenbun requested files without returning any file requests"
            )
        if self._current.status == "complete" and self.file_requests:
            raise RuntimeError("Kenbun returned file requests for a complete analysis")


def remote_analysis(
    files: Iterable[FileEntry],
    *,
    ecosystems: Iterable[Ecosystem] | None = None,
    inventory_complete: bool = True,
    hints: AnalysisHints | None = None,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    max_files: int | None = None,
    max_file_bytes: int = DEFAULT_MAX_FILE_BYTES,
) -> RemoteAnalysis:
    return RemoteAnalysis(
        files,
        ecosystems=ecosystems,
        inventory_complete=inventory_complete,
        hints=hints,
        max_rounds=max_rounds,
        max_files=max_files,
        max_file_bytes=max_file_bytes,
    )


__all__ = ["RemoteAnalysis", "remote_analysis"]
