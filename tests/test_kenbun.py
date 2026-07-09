import importlib
import importlib.machinery

import pytest


def test_hello_returns_hello_world():
    try:
        kenbun = importlib.import_module("kenbun")
    except ModuleNotFoundError:
        pytest.fail("kenbun should be importable")

    assert kenbun.hello() == "Hello, world!"


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
    assert kenbun.hello is extension.hello
