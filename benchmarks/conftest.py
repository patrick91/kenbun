from pathlib import Path

import pytest

import kenbun

FIXTURES = Path(__file__).parent / "fixtures"
Repository = tuple[list[kenbun.FileEntry], dict[str, bytes]]


def repository_fixture(name: str) -> Repository:
    root = FIXTURES / name
    contents = {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in root.rglob("*")
        if path.is_file()
    }
    ordered_contents = dict(sorted(contents.items()))
    files = [
        kenbun.FileEntry(path=path, size=len(source))
        for path, source in ordered_contents.items()
    ]
    return files, ordered_contents


@pytest.fixture(scope="session")
def fastapi_service_fixture() -> Repository:
    return repository_fixture("fastapi_service")


@pytest.fixture(scope="session")
def complex_workspace_fixture() -> Repository:
    return repository_fixture("complex_workspace")
