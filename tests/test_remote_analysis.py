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


def test_remote_analysis_preserves_selected_ecosystems_across_rounds() -> None:
    ecosystems = (ecosystem for ecosystem in ["python"])
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml"), entry("package.json")],
        ecosystems=ecosystems,
    )

    assert [request.path for request in analysis.file_requests] == ["pyproject.toml"]

    analysis.update({"pyproject.toml": FASTAPI_MANIFEST})

    application = analysis.result.applications[0]
    assert {item.ecosystem for item in application.dependencies} == {"python"}
    assert application.node is None


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


def test_remote_analysis_stops_at_the_round_limit() -> None:
    """Running out of rounds narrows the result rather than failing it, the same
    way `max_files` and `max_file_bytes` do. What was read still describes the
    repository, and callers should not have to choose between it and nothing."""
    files = [entry(f"packages/{index:02}/pyproject.toml") for index in range(65)]
    analysis = kenbun.remote_analysis(files, max_rounds=1)

    analysis.update(
        {
            request.path: b'[project]\nname = "library"\n'
            for request in analysis.file_requests
        }
    )

    assert analysis.round_limit_reached is True
    assert analysis.file_requests == []
    assert analysis.round_number == 1
    # Reachable without re-running the analysis, and honest about being partial.
    assert analysis.result.completeness == "partial"
    # `status` stays "needs_files": it reports that the analysis still wanted
    # more, which is true. The session decided to stop, and says so separately.
    assert analysis.result.status == "needs_files"


def test_remote_analysis_within_the_round_limit_is_complete() -> None:
    contents = {"pyproject.toml": FASTAPI_MANIFEST, "app.py": FASTAPI_APP}
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml"), entry("app.py")],
        hints={"script_patterns": ["app.py"]},
        max_rounds=5,
    )

    while analysis.file_requests:
        analysis.update(
            {request.path: contents[request.path] for request in analysis.file_requests}
        )

    assert analysis.round_limit_reached is False
    assert analysis.result.completeness == "complete"


def test_a_spent_round_budget_reports_the_session_as_complete() -> None:
    """A session that has stopped asking must not accept more content: the
    caller has nothing left to answer."""
    files = [entry(f"packages/{index:02}/pyproject.toml") for index in range(65)]
    analysis = kenbun.remote_analysis(files, max_rounds=1)
    analysis.update(
        {
            request.path: b'[project]\nname = "library"\n'
            for request in analysis.file_requests
        }
    )

    with pytest.raises(RuntimeError, match="already complete"):
        analysis.update({})


def test_remote_analysis_ignores_files_below_the_depth_limit() -> None:
    analysis = kenbun.remote_analysis(
        [
            entry("vendor/a/b/c/.gitignore"),
            entry("vendor/a/b/c/pyproject.toml"),
            entry("pyproject.toml"),
        ],
        max_depth=1,
    )

    assert [request.path for request in analysis.file_requests] == ["pyproject.toml"]

    analysis.update({"pyproject.toml": FASTAPI_MANIFEST})

    # Depth is an exclusion, not a budget that ran out. Calling it partial would
    # mark ordinary repositories incomplete over paths that could never have
    # held an application, drowning out the results that really are partial.
    assert analysis.result.completeness == "complete"


def test_remote_analysis_keeps_everything_within_the_depth_limit() -> None:
    analysis = kenbun.remote_analysis(
        [entry("services/api/pyproject.toml")],
        max_depth=2,
    )

    assert [request.path for request in analysis.file_requests] == [
        "services/api/pyproject.toml"
    ]

    analysis.update({"services/api/pyproject.toml": FASTAPI_MANIFEST})

    assert analysis.result.completeness == "complete"


@pytest.mark.parametrize("max_rounds", [0, -1, True, 1.5])
def test_remote_analysis_requires_positive_round_limit(max_rounds: int) -> None:
    with pytest.raises(ValueError, match="positive integer"):
        kenbun.remote_analysis([], max_rounds=max_rounds)


@pytest.mark.parametrize("max_depth", [-1, True, 1.5])
def test_remote_analysis_requires_non_negative_depth_limit(max_depth: int) -> None:
    with pytest.raises(ValueError, match="non-negative integer or None"):
        kenbun.remote_analysis([], max_depth=max_depth)


@pytest.mark.parametrize("max_files", [-1, True, 1.5])
def test_remote_analysis_requires_non_negative_file_limit(max_files: int) -> None:
    with pytest.raises(ValueError, match="non-negative integer or None"):
        kenbun.remote_analysis([], max_files=max_files)


@pytest.mark.parametrize("max_file_bytes", [0, -1, True, 1.5])
def test_remote_analysis_requires_positive_file_limit(max_file_bytes: int) -> None:
    with pytest.raises(ValueError, match="positive integer"):
        kenbun.remote_analysis([], max_file_bytes=max_file_bytes)


def test_remote_analysis_rejects_a_bare_ecosystem_string() -> None:
    with pytest.raises(TypeError) as error:
        kenbun.remote_analysis([], ecosystems="python")  # type: ignore[arg-type]
    assert str(error.value) == (
        "ecosystems must be an iterable of ecosystem names, not a string"
    )


LFS_POINTER = (
    b"version https://git-lfs.github.com/spec/v1\n"
    b"oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n"
    b"size 12345\n"
)


def test_lfs_pointer_content_is_treated_as_unavailable() -> None:
    """Hosts serve pointers for LFS-tracked files, and the tree reports the
    pointer's size, so they cannot be filtered out before the fetch."""
    analysis = kenbun.remote_analysis(
        [entry("requirements.txt", size=len(LFS_POINTER))],
    )

    analysis.update({"requirements.txt": LFS_POINTER})

    # Passing the pointer through would let a line-oriented manifest parse
    # cleanly and report a completeness it has not earned.
    assert analysis.result.completeness == "partial"


def test_lfs_pointer_ignore_file_cannot_produce_a_complete_result() -> None:
    analysis = kenbun.remote_analysis(
        [
            entry(".gitignore", size=len(LFS_POINTER)),
            entry("ignored/pyproject.toml"),
        ],
    )

    analysis.update({".gitignore": LFS_POINTER})
    analysis.update({"ignored/pyproject.toml": FASTAPI_MANIFEST})

    assert analysis.result.completeness == "partial"
    assert [
        application.application_dir for application in analysis.result.applications
    ] == ["ignored"]
    assert any(
        diagnostic.code == "KB801" and diagnostic.path == ".gitignore"
        for diagnostic in analysis.result.diagnostics
    )


def test_real_content_still_completes() -> None:
    analysis = kenbun.remote_analysis(
        [entry("pyproject.toml", size=len(FASTAPI_MANIFEST))],
    )

    analysis.update({"pyproject.toml": FASTAPI_MANIFEST})

    assert analysis.result.completeness == "complete"
