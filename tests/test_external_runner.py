import importlib.util
import io
import json
import tarfile
from pathlib import Path
from types import ModuleType

import pytest


def load_runner() -> ModuleType:
    path = Path(__file__).parents[1] / "scripts" / "check_external_fixtures.py"
    spec = importlib.util.spec_from_file_location("check_external_fixtures", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_manifest_expectation_reuse(tmp_path: Path) -> None:
    runner = load_runner()
    revision = "a" * 40
    manifest = tmp_path / "fixtures.json"
    manifest.write_text(
        json.dumps(
            [
                {
                    "name": "root",
                    "repository": "test-patrick/root",
                    "revision": revision,
                    "expected": {"applications": []},
                },
                {
                    "name": "member",
                    "repository": "test-patrick/root",
                    "revision": revision,
                    "expected_from": "root",
                    "expected_overrides": {"scan_origin": "apps/web"},
                },
            ]
        )
    )
    fixtures = runner.load_manifest(manifest)
    assert fixtures[1]["expected"] == {
        "applications": [],
        "scan_origin": "apps/web",
    }


def test_manifest_requires_full_commit_sha(tmp_path: Path) -> None:
    runner = load_runner()
    manifest = tmp_path / "fixtures.json"
    manifest.write_text(
        json.dumps(
            [
                {
                    "name": "mutable",
                    "repository": "test-patrick/root",
                    "revision": "main",
                    "expected": {},
                }
            ]
        )
    )
    with pytest.raises(ValueError, match="full commit SHA"):
        runner.load_manifest(manifest)


def test_safe_extract_rejects_links(tmp_path: Path) -> None:
    runner = load_runner()
    archive = tmp_path / "fixture.tar.gz"
    with tarfile.open(archive, "w:gz") as bundle:
        directory = tarfile.TarInfo("fixture")
        directory.type = tarfile.DIRTYPE
        directory.mode = 0o755
        bundle.addfile(directory)
        link = tarfile.TarInfo("fixture/link")
        link.type = tarfile.SYMTYPE
        link.linkname = "../../outside"
        bundle.addfile(link)
    with pytest.raises(RuntimeError, match="unsupported archive member"):
        runner.safe_extract(archive, tmp_path / "extracted")


def test_safe_extract_accepts_regular_archive(tmp_path: Path) -> None:
    runner = load_runner()
    archive = tmp_path / "fixture.tar.gz"
    content = b"hello\n"
    with tarfile.open(archive, "w:gz") as bundle:
        directory = tarfile.TarInfo("fixture")
        directory.type = tarfile.DIRTYPE
        directory.mode = 0o755
        bundle.addfile(directory)
        file = tarfile.TarInfo("fixture/README.md")
        file.mode = 0o644
        file.size = len(content)
        bundle.addfile(file, io.BytesIO(content))
    root = runner.safe_extract(archive, tmp_path / "extracted")
    assert (root / "README.md").read_bytes() == content


def test_safe_extract_replaces_incomplete_cached_directory(tmp_path: Path) -> None:
    runner = load_runner()
    archive = tmp_path / "fixture.tar.gz"
    content = b"fresh\n"
    with tarfile.open(archive, "w:gz") as bundle:
        directory = tarfile.TarInfo("fixture")
        directory.type = tarfile.DIRTYPE
        directory.mode = 0o755
        bundle.addfile(directory)
        file = tarfile.TarInfo("fixture/README.md")
        file.mode = 0o644
        file.size = len(content)
        bundle.addfile(file, io.BytesIO(content))

    stale_root = tmp_path / "extracted" / "fixture"
    stale_root.mkdir(parents=True)
    (stale_root / "README.md").write_text("stale\n")

    root = runner.safe_extract(archive, tmp_path / "extracted")
    assert (root / "README.md").read_bytes() == content
