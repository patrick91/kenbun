"""Run Kenbun against immutable GitHub fixture snapshots.

This is intentionally a manual acceptance check. It never installs or executes
fixture code, and the normal test suite never imports this module.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import sys
import tarfile
import urllib.request
from pathlib import Path
from typing import Any

import kenbun

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "tests" / "external_fixtures.json"
DEFAULT_CACHE = ROOT / "target" / "github-fixtures"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument(
        "--fixture",
        action="append",
        default=[],
        help="Run only a named fixture (repeatable)",
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="Use cached archives only; fail if one is missing",
    )
    parser.add_argument(
        "--refresh",
        action="store_true",
        help="Discard cached archives before downloading",
    )
    return parser.parse_args()


def load_manifest(path: Path) -> list[dict[str, Any]]:
    data = json.loads(path.read_text())
    if not isinstance(data, list):
        raise ValueError(f"{path} must contain a JSON list")
    required = {"name", "repository", "revision"}
    for fixture in data:
        missing = required - fixture.keys()
        if missing:
            raise ValueError(f"fixture is missing {sorted(missing)}: {fixture!r}")
        revision = fixture["revision"]
        if len(revision) != 40 or any(c not in "0123456789abcdef" for c in revision):
            raise ValueError(f"fixture {fixture['name']} must use a full commit SHA")
        if "expected" not in fixture and "expected_from" not in fixture:
            raise ValueError(
                f"fixture {fixture['name']} needs `expected` or `expected_from`"
            )
    by_name = {fixture["name"]: fixture for fixture in data}
    for fixture in data:
        if source_name := fixture.get("expected_from"):
            source = by_name.get(source_name)
            if source is None or "expected" not in source:
                raise ValueError(
                    f"fixture {fixture['name']} references unknown expectations: "
                    f"{source_name}"
                )
            fixture["expected"] = copy.deepcopy(source["expected"])
            fixture["expected"].update(fixture.get("expected_overrides", {}))
    return data


def download_archive(fixture: dict[str, Any], cache_dir: Path, offline: bool) -> Path:
    repository = fixture["repository"]
    revision = fixture["revision"]
    archive = cache_dir / repository / f"{revision}.tar.gz"
    if archive.exists():
        return archive
    if offline:
        raise RuntimeError(f"not cached: {repository}@{revision}")

    archive.parent.mkdir(parents=True, exist_ok=True)
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repository}/tarball/{revision}",
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "kenbun-external-fixtures",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    if token := os.environ.get("GITHUB_TOKEN"):
        request.add_header("Authorization", f"Bearer {token}")

    temporary = archive.with_suffix(".tmp")
    try:
        with (
            urllib.request.urlopen(request) as response,
            temporary.open("wb") as output,
        ):
            shutil.copyfileobj(response, output)
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    return archive


def safe_extract(archive: Path, destination: Path) -> Path:
    completion_marker = destination / ".kenbun-extraction-complete"
    if destination.exists():
        roots = [path for path in destination.iterdir() if path.is_dir()]
        if (
            len(roots) == 1
            and completion_marker.is_file()
            and completion_marker.read_text().strip() == archive.name
        ):
            return roots[0]
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    root = destination.resolve()
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle.getmembers():
            target = (destination / member.name).resolve()
            if not target.is_relative_to(root):
                raise RuntimeError(f"unsafe archive path: {member.name}")
            if (
                member.issym()
                or member.islnk()
                or not (member.isfile() or member.isdir())
            ):
                raise RuntimeError(f"unsupported archive member: {member.name}")
        if sys.version_info >= (3, 12):
            bundle.extractall(destination, filter="data")
        else:  # pragma: no cover - exercised by the Python 3.10/3.11 CI jobs
            bundle.extractall(destination)
    roots = [path for path in destination.iterdir() if path.is_dir()]
    if len(roots) != 1:
        raise RuntimeError(
            f"expected one archive root in {archive}, found {len(roots)}"
        )
    completion_marker.write_text(f"{archive.name}\n")
    return roots[0]


def projection(result: Any) -> dict[str, Any]:
    workspace = None
    if result.workspace is not None:
        workspace = {
            "kind": result.workspace.kind,
            "path": result.workspace.path,
            "virtual_root": result.workspace.virtual_root,
            "members": list(result.workspace.members),
        }

    applications = []
    for application in result.applications:
        applications.append(
            {
                "application_dir": application.application_dir,
                "technologies": sorted(
                    [
                        ":".join(
                            [
                                technology.kind,
                                technology.role,
                                technology.name,
                                technology.confidence,
                            ]
                        )
                        for technology in application.technologies
                    ],
                ),
                "entrypoint": (
                    application.entrypoint.as_string
                    if application.entrypoint is not None
                    else None
                ),
                "dependency_managers": {
                    dependencies.ecosystem: dependencies.package_manager
                    for dependencies in application.dependencies
                },
                "build_scripts": [
                    [
                        script.package_manager,
                        script.command,
                        list(script.argv) if script.argv is not None else None,
                    ]
                    for script in application.build_scripts
                ],
                "diagnostic_codes": sorted(
                    diagnostic.code for diagnostic in application.diagnostics
                ),
            }
        )
    applications.sort(key=lambda item: item["application_dir"])
    return {
        "schema_version": result.schema_version,
        "upload_root": result.upload_root,
        "scan_origin": result.scan_origin,
        "workspace": workspace,
        "applications": applications,
        "diagnostic_codes": sorted(
            diagnostic.code for diagnostic in result.diagnostics
        ),
    }


def expected_projection(expected: dict[str, Any]) -> dict[str, Any]:
    expanded = {
        "schema_version": 1,
        "upload_root": ".",
        "scan_origin": ".",
        "workspace": None,
        "applications": [],
        "diagnostic_codes": [],
    }
    expanded.update(expected)
    applications = []
    for application in expanded["applications"]:
        item = {
            "entrypoint": None,
            "dependency_managers": {},
            "build_scripts": [],
            "diagnostic_codes": [],
        }
        item.update(application)
        item["technologies"] = sorted(item["technologies"])
        applications.append(item)
    expanded["applications"] = sorted(
        applications, key=lambda application: application["application_dir"]
    )
    return expanded


def run_fixture(
    fixture: dict[str, Any], cache_dir: Path, offline: bool, refresh: bool
) -> tuple[dict[str, Any], dict[str, Any]]:
    repository_cache = cache_dir / fixture["repository"]
    archive = repository_cache / f"{fixture['revision']}.tar.gz"
    extracted = repository_cache / fixture["revision"]
    if refresh:
        archive.unlink(missing_ok=True)
        shutil.rmtree(extracted, ignore_errors=True)
    archive = download_archive(fixture, cache_dir, offline)
    archive_root = safe_extract(archive, extracted)
    scan_root = archive_root / fixture.get("scan_subdirectory", ".")
    scan_arguments = fixture.get("scan_arguments", {})
    result = kenbun.scan(scan_root, **scan_arguments)
    return projection(result), expected_projection(fixture["expected"])


def main() -> int:
    args = parse_args()
    fixtures = load_manifest(args.manifest)
    selected = set(args.fixture)
    known = {fixture["name"] for fixture in fixtures}
    if unknown := selected - known:
        raise SystemExit(f"unknown fixtures: {', '.join(sorted(unknown))}")
    if selected:
        fixtures = [fixture for fixture in fixtures if fixture["name"] in selected]

    failures = 0
    for fixture in fixtures:
        try:
            actual, expected = run_fixture(
                fixture, args.cache_dir, args.offline, args.refresh
            )
        except Exception as error:
            failures += 1
            print(f"FAIL {fixture['name']}: {error}", file=sys.stderr)
            continue
        if actual == expected:
            print(f"PASS {fixture['name']}")
            continue
        failures += 1
        print(f"FAIL {fixture['name']}", file=sys.stderr)
        print(
            json.dumps({"expected": expected, "actual": actual}, indent=2),
            file=sys.stderr,
        )
    print(f"{len(fixtures) - failures} passed; {failures} failed")
    return int(failures != 0)


if __name__ == "__main__":
    raise SystemExit(main())
