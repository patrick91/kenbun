from __future__ import annotations

import pytest

import kenbun

FASTAPI_MANIFEST = b"""[project]
name = "demo"
dependencies = ["fastapi"]
"""
FASTAPI_APP = b"from fastapi import FastAPI\napp = FastAPI()\n"


def entry(path: str, *, size: int | None = None) -> kenbun.FileEntry:
    return kenbun.FileEntry(path=path, size=size)


def test_remote_analysis_drives_incremental_analysis() -> None:
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml"), entry("app.py")],
        hints={"script_patterns": ["app.py"]},
    )

    assert analysis.round_number == 1
    assert [request.path for request in analysis.file_requests] == ["pyproject.toml"]
    with pytest.raises(RuntimeError, match="not complete"):
        _ = analysis.result

    analysis.update({"pyproject.toml": FASTAPI_MANIFEST})

    assert analysis.round_number == 2
    assert [request.path for request in analysis.file_requests] == ["app.py"]

    analysis.update({"app.py": FASTAPI_APP})

    assert analysis.file_requests == []
    assert analysis.result.applications[0].entrypoint.as_string == "app:app"


def test_remote_analysis_accepts_unavailable_content() -> None:
    analysis = kenbun.remote_analysis([entry("pyproject.toml")])

    analysis.update({"pyproject.toml": None})

    assert analysis.file_requests == []
    assert analysis.result.completeness == "partial"


def test_remote_analysis_validates_updates() -> None:
    analysis = kenbun.remote_analysis([entry("pyproject.toml")])

    with pytest.raises(ValueError, match="Missing responses"):
        analysis.update({})
    with pytest.raises(ValueError, match="unrequested"):
        analysis.update(
            {
                "pyproject.toml": FASTAPI_MANIFEST,
                "unexpected.py": b"",
            }
        )
    with pytest.raises(TypeError, match="bytes or None"):
        analysis.update({"pyproject.toml": "not bytes"})  # type: ignore[dict-item]
    with pytest.raises(ValueError, match="exceeds"):
        analysis.update({"pyproject.toml": b"x" * (2 * 1024 * 1024 + 1)})

    analysis.update({"pyproject.toml": FASTAPI_MANIFEST})
    with pytest.raises(RuntimeError, match="already complete"):
        analysis.update({})


def test_remote_analysis_skips_known_oversized_files() -> None:
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml", size=5)],
        max_file_bytes=4,
    )

    assert analysis.file_requests == []
    assert analysis.result.completeness == "partial"


@pytest.mark.parametrize("size", [None, 4])
def test_remote_analysis_validates_fetchable_size_after_fetch(
    size: int | None,
) -> None:
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml", size=size)],
        max_file_bytes=4,
    )

    with pytest.raises(ValueError, match="4-byte limit"):
        analysis.update({"pyproject.toml": b"12345"})


def test_remote_analysis_applies_file_count_limit_across_rounds() -> None:
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml"), entry("app.py")],
        hints={"script_patterns": ["app.py"]},
        max_files=1,
    )

    assert [request.path for request in analysis.file_requests] == ["pyproject.toml"]

    analysis.update({"pyproject.toml": FASTAPI_MANIFEST})

    assert analysis.file_requests == []
    assert analysis.result.completeness == "partial"
    assert analysis.result.applications[0].entrypoint is None


def test_remote_analysis_enforces_round_limit() -> None:
    files = [entry(f"packages/{index:02}/pyproject.toml") for index in range(65)]
    analysis = kenbun.remote_analysis(files, max_rounds=1)

    with pytest.raises(RuntimeError, match="within 1 rounds"):
        analysis.update(
            {
                request.path: b'[project]\nname = "library"\n'
                for request in analysis.file_requests
            }
        )


@pytest.mark.parametrize("max_rounds", [0, -1, True, 1.5])
def test_remote_analysis_requires_positive_round_limit(max_rounds: int) -> None:
    with pytest.raises(ValueError, match="positive integer"):
        kenbun.remote_analysis([], max_rounds=max_rounds)


@pytest.mark.parametrize("max_files", [-1, True, 1.5])
def test_remote_analysis_requires_non_negative_file_limit(max_files: int) -> None:
    with pytest.raises(ValueError, match="non-negative integer or None"):
        kenbun.remote_analysis([], max_files=max_files)


@pytest.mark.parametrize("max_file_bytes", [0, -1, True, 1.5])
def test_remote_analysis_requires_positive_file_limit(max_file_bytes: int) -> None:
    with pytest.raises(ValueError, match="positive integer"):
        kenbun.remote_analysis([], max_file_bytes=max_file_bytes)
