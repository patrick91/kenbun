import importlib
import importlib.machinery

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
