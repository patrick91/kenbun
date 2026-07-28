from __future__ import annotations

import pytest

import kenbun

FASTAPI_MANIFEST = b"""[project]
name = "demo"
dependencies = ["fastapi"]
"""
FASTAPI_APP = b"from fastapi import FastAPI\napp = FastAPI()\n"


def test_remote_analysis_drives_incremental_analysis() -> None:
    analysis = kenbun.remote_analysis(
        ["pyproject.toml", "app.py"],
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
    analysis = kenbun.remote_analysis(["pyproject.toml"])

    analysis.update({"pyproject.toml": None})

    assert analysis.file_requests == []
    assert analysis.result.completeness == "partial"


def test_remote_analysis_validates_updates() -> None:
    analysis = kenbun.remote_analysis(["pyproject.toml"])

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


def test_remote_analysis_applies_custom_file_limit() -> None:
    analysis = kenbun.remote_analysis(["pyproject.toml"], max_file_bytes=4)

    request = analysis.file_requests[0]
    assert analysis.should_fetch(request, None)
    assert analysis.should_fetch(request, 4)
    assert not analysis.should_fetch(request, 5)
    with pytest.raises(ValueError, match="4-byte limit"):
        analysis.update({"pyproject.toml": b"12345"})


@pytest.mark.parametrize("size", [-1, True, 1.5])
def test_remote_analysis_should_fetch_requires_valid_size(size: int) -> None:
    analysis = kenbun.remote_analysis(["pyproject.toml"])

    with pytest.raises(ValueError, match="non-negative integer or None"):
        analysis.should_fetch(analysis.file_requests[0], size)


def test_remote_analysis_should_fetch_requires_pending_request() -> None:
    first = kenbun.remote_analysis(["pyproject.toml"])
    stale_request = first.file_requests[0]
    first.update({"pyproject.toml": FASTAPI_MANIFEST})

    with pytest.raises(ValueError, match="not pending"):
        first.should_fetch(stale_request, 1)

    second = kenbun.remote_analysis(["app.py"])
    with pytest.raises(ValueError, match="not pending"):
        second.should_fetch(stale_request, 1)


def test_remote_analysis_enforces_round_limit() -> None:
    files = [f"packages/{index:02}/pyproject.toml" for index in range(65)]
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


@pytest.mark.parametrize("max_file_bytes", [0, -1, True, 1.5])
def test_remote_analysis_requires_positive_file_limit(max_file_bytes: int) -> None:
    with pytest.raises(ValueError, match="positive integer"):
        kenbun.remote_analysis([], max_file_bytes=max_file_bytes)
