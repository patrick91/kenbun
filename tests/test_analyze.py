from __future__ import annotations

from types import MappingProxyType

import pytest

import kenbun

FASTAPI_MANIFEST = b"""[project]
name = "demo"
dependencies = ["fastapi"]
"""
FASTAPI_APP = b"from fastapi import FastAPI\napp = FastAPI()\n"


def entry(path: str, *, size: int | None = None) -> kenbun.FileEntry:
    return kenbun.FileEntry(path=path, size=size)


def test_manifest_only_pass_does_not_request_scripts() -> None:
    files = [entry("pyproject.toml"), entry("app.py")]

    first = kenbun.analyze(files)
    assert first.status == "needs_files"
    assert first.completeness == "partial"
    assert [(request.path, request.priority) for request in first.file_requests] == [
        ("pyproject.toml", 10)
    ]

    result = kenbun.analyze(files, {"pyproject.toml": FASTAPI_MANIFEST})
    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.file_requests == []
    assert len(result.applications) == 1
    assert result.applications[0].name == "demo"
    assert result.applications[0].entrypoint is None


def test_analyze_accepts_generic_iterables_and_mappings() -> None:
    files = (entry(path) for path in ["pyproject.toml"])
    contents = MappingProxyType({"pyproject.toml": FASTAPI_MANIFEST})

    result = kenbun.analyze(files, contents)

    assert result.status == "complete"
    assert result.applications[0].name == "demo"


def test_analyze_applies_custom_file_limit() -> None:
    result = kenbun.analyze(
        [entry("pyproject.toml")],
        {"pyproject.toml": b"12345"},
        max_file_bytes=4,
    )

    assert result.status == "complete"
    assert result.completeness == "partial"


def test_analyze_skips_known_oversized_files() -> None:
    result = kenbun.analyze(
        [entry("pyproject.toml", size=5)],
        max_file_bytes=4,
    )

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.file_requests == []
    assert any(
        diagnostic.code == "KB801" and "4-byte parse cap" in diagnostic.message
        for diagnostic in result.diagnostics
    )


def test_analyze_applies_file_count_limit() -> None:
    result = kenbun.analyze([entry("pyproject.toml")], max_files=0)

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.file_requests == []


def test_depth_pruned_contents_do_not_spend_the_file_budget() -> None:
    result = kenbun.analyze(
        [entry("pyproject.toml"), entry("vendor/a/b/pyproject.toml")],
        {"vendor/a/b/pyproject.toml": FASTAPI_MANIFEST},
        max_files=1,
        max_depth=0,
    )

    assert [request.path for request in result.file_requests] == ["pyproject.toml"]


def test_script_hints_drive_incremental_entrypoint_resolution() -> None:
    files = [entry("pyproject.toml"), entry("services/api/app.py")]
    contents = {"pyproject.toml": FASTAPI_MANIFEST}
    hints = {"script_patterns": ["app.py"]}

    requested = kenbun.analyze(files, contents, hints=hints)
    assert [
        (request.path, request.priority) for request in requested.file_requests
    ] == [("services/api/app.py", 40)]

    contents["services/api/app.py"] = FASTAPI_APP
    result = kenbun.analyze(files, contents, hints=hints)
    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.applications[0].entrypoint.as_string == "app:app"


def test_script_hints_are_ordered_patterns_and_batched() -> None:
    files = [
        entry("pyproject.toml"),
        *(entry(f"scripts/script_{index:02}.py") for index in range(20)),
    ]
    manifest = {"pyproject.toml": FASTAPI_MANIFEST}

    first = kenbun.analyze(
        files,
        manifest,
        hints={"script_patterns": ["**/*.py"]},
    )
    assert first.status == "needs_files"
    assert len(first.file_requests) == 16

    contents = {
        **manifest,
        **{request.path: b"print('ok')\n" for request in first.file_requests},
    }
    second = kenbun.analyze(
        files,
        contents,
        hints={"script_patterns": ["**/*.py"]},
    )
    assert len(second.file_requests) == 4
    assert not set(contents).intersection(
        request.path for request in second.file_requests
    )


def test_script_hints_do_not_bypass_the_manifest_quick_pass() -> None:
    files = [entry("app.py")]

    result = kenbun.analyze(files, hints={"script_patterns": ["app.py"]})

    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.file_requests == []


def test_non_framework_manifest_does_not_request_scripts() -> None:
    files = [entry("pyproject.toml"), entry("app.py")]

    result = kenbun.analyze(
        files,
        {"pyproject.toml": b'[project]\nname = "library"\n'},
        hints={"script_patterns": ["app.py"]},
    )

    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.file_requests == []


def test_lockfiles_are_not_requested() -> None:
    files = [entry("pyproject.toml"), entry("uv.lock")]

    result = kenbun.analyze(files, {"pyproject.toml": FASTAPI_MANIFEST})

    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.file_requests == []


def test_lockfile_inventory_infers_manager_without_requesting_contents() -> None:
    files = [entry("package.json"), entry("package-lock.json")]
    package = b"""{
      "dependencies": {"next": "16.0.0", "react": "19.0.0"},
      "scripts": {"build": "next build"}
    }"""

    first = kenbun.analyze(files)
    assert [request.path for request in first.file_requests] == ["package.json"]

    result = kenbun.analyze(files, {"package.json": package})

    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.file_requests == []
    assert result.applications[0].dependencies[0].package_manager == "npm"
    assert result.applications[0].dependencies[0].package_manager_version is None
    assert result.applications[0].build_scripts[0].package_manager == "npm"


def test_analyze_requests_and_reports_only_selected_ecosystems() -> None:
    files = [entry("pyproject.toml"), entry("package.json")]
    package = b'{"dependencies": {"next": "16", "react": "19"}}'

    python_first = kenbun.analyze(files, ecosystems={"python"})
    assert [request.path for request in python_first.file_requests] == [
        "pyproject.toml"
    ]
    python_result = kenbun.analyze(
        files,
        {"pyproject.toml": FASTAPI_MANIFEST},
        ecosystems={"python"},
    )
    assert python_result.status == "complete"
    assert {
        dependency.ecosystem
        for dependency in python_result.applications[0].dependencies
    } == {"python"}
    assert python_result.applications[0].node is None

    node_first = kenbun.analyze(files, ecosystems=("node",))
    assert [request.path for request in node_first.file_requests] == ["package.json"]
    node_result = kenbun.analyze(
        files,
        {"package.json": package},
        ecosystems=("node",),
    )
    assert node_result.status == "complete"
    assert {
        dependency.ecosystem for dependency in node_result.applications[0].dependencies
    } == {"node"}
    assert node_result.applications[0].python is None


def test_unavailable_content_terminates_with_partial_result() -> None:
    files = [entry("pyproject.toml")]

    result = kenbun.analyze(files, {"pyproject.toml": None})

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.file_requests == []


def test_invalid_utf8_content_is_partial() -> None:
    files = [entry("pyproject.toml")]

    result = kenbun.analyze(files, {"pyproject.toml": b"\xff"})

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.file_requests == []


def test_malformed_identity_manifest_is_partial() -> None:
    files = [entry("pyproject.toml")]

    result = kenbun.analyze(files, {"pyproject.toml": b"not = [valid"})

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert {diagnostic.code for diagnostic in result.diagnostics} >= {"KB201"}


def test_incomplete_inventory_prevents_definitive_negative() -> None:
    result = kenbun.analyze([], inventory_complete=False)

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.applications == []


def test_ignore_files_are_requested_before_manifests_and_filter_inventory() -> None:
    files = [
        entry(".gitignore"),
        entry("ignored/pyproject.toml"),
    ]

    first = kenbun.analyze(files)
    assert [request.path for request in first.file_requests] == [".gitignore"]

    result = kenbun.analyze(files, {".gitignore": b"ignored/\n"})
    assert result.status == "complete"
    assert result.file_requests == []
    assert result.applications == []


def test_nested_ignore_files_filter_only_their_subtree() -> None:
    files = [
        entry("services/.gitignore"),
        entry("services/generated/pyproject.toml"),
        entry("generated/pyproject.toml"),
    ]
    contents = {
        "services/.gitignore": b"generated/\n",
        "generated/pyproject.toml": FASTAPI_MANIFEST,
    }

    result = kenbun.analyze(files, contents)

    assert result.status == "complete"
    assert [application.application_dir for application in result.applications] == [
        "generated"
    ]


def test_manifest_requests_continue_past_the_first_batch() -> None:
    files = [entry(f"packages/{index:02}/pyproject.toml") for index in range(65)]

    first = kenbun.analyze(files)
    assert len(first.file_requests) == 64

    contents = {
        request.path: b'[project]\nname = "library"\n'
        for request in first.file_requests
    }
    second = kenbun.analyze(files, contents)
    assert [request.path for request in second.file_requests] == [
        "packages/64/pyproject.toml"
    ]

    contents["packages/64/pyproject.toml"] = FASTAPI_MANIFEST
    result = kenbun.analyze(files, contents)
    assert result.status == "complete"
    assert [application.application_dir for application in result.applications] == [
        "packages/64"
    ]


def test_invalid_inputs_fail_loudly() -> None:
    with pytest.raises(TypeError, match=r"files\[0\]\.path must be a string"):
        kenbun.analyze([("pyproject.toml", 64)])
    with pytest.raises(TypeError, match=r"files\[0\]\.path must be a string"):
        kenbun.analyze([{"size": 64}])
    with pytest.raises(TypeError, match=r"files\[0\]\.size must be"):
        kenbun.analyze([{"path": "pyproject.toml"}])
    for size in (-1, True, 1.5):
        with pytest.raises(TypeError, match=r"files\[0\]\.size must be"):
            kenbun.analyze([{"path": "pyproject.toml", "size": size}])
    with pytest.raises(ValueError, match="repository-relative"):
        kenbun.analyze([entry("../pyproject.toml")])
    with pytest.raises(ValueError, match="unknown analysis hint"):
        kenbun.analyze([], hints={"scripts_patterns": ["app.py"]})
    with pytest.raises(ValueError, match="invalid script pattern"):
        kenbun.analyze([], hints={"script_patterns": ["../*.py"]})
    with pytest.raises(ValueError, match="positive integer"):
        kenbun.analyze([], max_file_bytes=0)
    for max_depth in (-1, True, 1.5):
        with pytest.raises(ValueError, match="non-negative integer or None"):
            kenbun.analyze([], max_depth=max_depth)
    with pytest.raises(TypeError) as error:
        kenbun.analyze([], ecosystems="python")
    assert str(error.value) == (
        "ecosystems must be an iterable of ecosystem names, not a string"
    )
    with pytest.raises(TypeError) as error:
        kenbun.analyze([], ecosystems=["python", 1])
    assert str(error.value) == "ecosystems must contain only strings"
    with pytest.raises(ValueError) as error:
        kenbun.analyze([], ecosystems=[])
    assert str(error.value) == (
        "ecosystems must contain at least one of: 'python', 'node'"
    )
    with pytest.raises(ValueError) as error:
        kenbun.analyze([], ecosystems=["ruby"])
    assert str(error.value) == "unknown ecosystem 'ruby'; expected 'python' or 'node'"
