import importlib
import importlib.machinery
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest


def test_kenbun_is_importable():
    try:
        kenbun = importlib.import_module("kenbun")
    except ModuleNotFoundError:
        pytest.fail("kenbun should be importable")

    assert callable(kenbun.scan)


def test_kenbun_is_a_mixed_python_rust_package():
    kenbun = importlib.import_module("kenbun")
    extension = importlib.import_module("kenbun._kenbun")

    assert kenbun.__file__ is not None
    assert kenbun.__file__.endswith("__init__.py")
    assert hasattr(kenbun, "__path__")
    assert any(
        extension.__file__.endswith(suffix)
        for suffix in importlib.machinery.EXTENSION_SUFFIXES
    )
    assert kenbun.scan is extension.scan
    assert kenbun.Application is extension.Application
    assert kenbun.NodeInfo is extension.NodeInfo


def test_scans_are_safe_to_run_concurrently(tmp_path: Path) -> None:
    kenbun = importlib.import_module("kenbun")
    (tmp_path / "requirements.txt").write_text("fastapi\n")
    (tmp_path / "main.py").write_text("from fastapi import FastAPI\napp = FastAPI()\n")

    def scan(_: int) -> str:
        return kenbun.scan(tmp_path).to_json()

    with ThreadPoolExecutor(max_workers=8) as executor:
        results = list(executor.map(scan, range(64)))
    assert len(set(results)) == 1
