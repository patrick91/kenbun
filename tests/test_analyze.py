from __future__ import annotations

from types import MappingProxyType

import pytest

import kenbun

FASTAPI_MANIFEST = b'''[project]
name = "demo"
dependencies = ["fastapi"]
'''
FASTAPI_APP = b"from fastapi import FastAPI\napp = FastAPI()\n"


def entry(path: str, size: int = 64) -> kenbun.FileEntry:
    return kenbun.FileEntry(path, size, f"sha-{path}")


def test_manifest_only_pass_does_not_request_scripts() -> None:
    files = [entry("pyproject.toml"), entry("app.py")]

    first = kenbun.analyze(files)
    assert first.status == "needs_files"
    assert first.completeness == "partial"
    assert [(want.path, want.priority, want.blob_sha) for want in first.want_files] == [
        ("pyproject.toml", 10, "sha-pyproject.toml")
    ]

    result = kenbun.analyze(files, {"pyproject.toml": FASTAPI_MANIFEST})
    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.want_files == []
    assert len(result.applications) == 1
    assert result.applications[0].name == "demo"
    assert result.applications[0].entrypoint is None


def test_analyze_accepts_generic_iterables_and_mappings() -> None:
    files = (entry(path) for path in ["pyproject.toml"])
    contents = MappingProxyType({"pyproject.toml": FASTAPI_MANIFEST})

    result = kenbun.analyze(files, contents)

    assert result.status == "complete"
    assert result.applications[0].name == "demo"


def test_script_hints_drive_incremental_entrypoint_resolution() -> None:
    files = [entry("pyproject.toml"), entry("services/api/app.py")]
    contents = {"pyproject.toml": FASTAPI_MANIFEST}
    hints = {"script_patterns": ["app.py"]}

    requested = kenbun.analyze(files, contents, hints=hints)
    assert [(want.path, want.priority) for want in requested.want_files] == [
        ("services/api/app.py", 40)
    ]

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
    assert len(first.want_files) == 16

    contents = {
        **manifest,
        **{want.path: b"print('ok')\n" for want in first.want_files},
    }
    second = kenbun.analyze(
        files,
        contents,
        hints={"script_patterns": ["**/*.py"]},
    )
    assert len(second.want_files) == 4
    assert not set(contents).intersection(want.path for want in second.want_files)


def test_script_hints_do_not_bypass_the_manifest_quick_pass() -> None:
    files = [entry("app.py")]

    result = kenbun.analyze(files, hints={"script_patterns": ["app.py"]})

    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.want_files == []


def test_non_framework_manifest_does_not_request_scripts() -> None:
    files = [entry("pyproject.toml"), entry("app.py")]

    result = kenbun.analyze(
        files,
        {"pyproject.toml": b'[project]\nname = "library"\n'},
        hints={"script_patterns": ["app.py"]},
    )

    assert result.status == "complete"
    assert result.completeness == "complete"
    assert result.want_files == []


def test_unavailable_content_terminates_with_partial_result() -> None:
    files = [entry("pyproject.toml")]

    result = kenbun.analyze(files, {"pyproject.toml": None})

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.want_files == []


def test_invalid_utf8_content_is_partial() -> None:
    files = [entry("pyproject.toml")]

    result = kenbun.analyze(files, {"pyproject.toml": b"\xff"})

    assert result.status == "complete"
    assert result.completeness == "partial"
    assert result.want_files == []


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
    assert [want.path for want in first.want_files] == [".gitignore"]

    result = kenbun.analyze(files, {".gitignore": b"ignored/\n"})
    assert result.status == "complete"
    assert result.want_files == []
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
    assert len(first.want_files) == 64

    contents = {
        want.path: b"[project]\nname = \"library\"\n" for want in first.want_files
    }
    second = kenbun.analyze(files, contents)
    assert [want.path for want in second.want_files] == [
        "packages/64/pyproject.toml"
    ]

    contents["packages/64/pyproject.toml"] = FASTAPI_MANIFEST
    result = kenbun.analyze(files, contents)
    assert result.status == "complete"
    assert [application.application_dir for application in result.applications] == [
        "packages/64"
    ]


def test_invalid_inputs_fail_loudly() -> None:
    with pytest.raises(ValueError, match="repository-relative"):
        kenbun.analyze([entry("../pyproject.toml")])
    with pytest.raises(ValueError, match="unknown analysis hint"):
        kenbun.analyze([], hints={"scripts_patterns": ["app.py"]})
    with pytest.raises(ValueError, match="invalid script pattern"):
        kenbun.analyze([], hints={"script_patterns": ["../*.py"]})
