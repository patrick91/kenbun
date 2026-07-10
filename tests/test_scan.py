"""Filesystem regression tests for kenbun.scan."""

from pathlib import Path

import pytest

import kenbun


def make(root: Path, files: dict[str, str]) -> None:
    for rel, content in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


FASTAPI_PYPROJECT = """\
[project]
name = "demo"
dependencies = ["fastapi[standard]>=0.115"]
"""

APP_MAIN = """\
from fastapi import FastAPI

app = FastAPI()
"""


def codes(result: kenbun.ScanResult) -> list[str]:
    return [d.code for d in result.diagnostics]


def app(result: kenbun.ScanResult, application_dir: str = ".") -> kenbun.Application:
    return next(
        application
        for application in result.applications
        if application.application_dir == application_dir
    )


def technology(application: kenbun.Application, name: str) -> kenbun.Technology:
    return next(item for item in application.technologies if item.name == name)


def dependencies(
    application: kenbun.Application, ecosystem: str = "python"
) -> kenbun.DependencySet:
    return next(
        item for item in application.dependencies if item.ecosystem == ecosystem
    )


# ── happy paths ─────────────────────────────────────────────────────────────


def test_single_app_high_confidence(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "app/main.py": APP_MAIN,
            "app/__init__.py": "",
        },
    )
    result = kenbun.scan(tmp_path)

    assert len(result.applications) == 1
    application = app(result)
    assert application.name == "demo"
    fastapi = technology(application, "fastapi")
    assert fastapi.kind == "framework"
    assert fastapi.role == "primary"
    assert fastapi.confidence == "high"
    python = technology(application, "python")
    assert python.kind == "language"
    assert python.role == "supporting"
    assert application.entrypoint is not None
    assert application.entrypoint.as_string == "app.main:app"
    assert application.entrypoint.source == "inferred"
    assert not application.entrypoint.is_factory


def test_flat_main_py(tmp_path: Path) -> None:
    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "main.py": APP_MAIN})
    result = kenbun.scan(tmp_path)
    assert app(result).entrypoint.as_string == "main:app"


def test_create_app_idiom_is_instance(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": (
                "from fastapi import FastAPI\n\n"
                "def create_app() -> FastAPI:\n"
                "    return FastAPI()\n\n"
                "app = create_app()\n"
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert application.entrypoint.as_string == "main:app"
    assert not application.entrypoint.is_factory
    assert technology(application, "fastapi").confidence == "high"


def test_factory_only_capped_kb112(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": (
                "from fastapi import FastAPI\n\n"
                "def create_app() -> FastAPI:\n"
                "    return FastAPI()\n"
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert application.entrypoint.is_factory
    assert application.entrypoint.attribute == "create_app"
    assert technology(application, "fastapi").confidence == "medium"
    assert "KB112" in codes(result)


def test_variable_precedence_app_over_api(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": (
                "from fastapi import FastAPI\n"
                "api = FastAPI()\n"
                "app = FastAPI()\n"
                "zz = FastAPI()\n"
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    assert app(result).entrypoint.attribute == "app"


def test_tool_fastapi_entrypoint_wins(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT
            + '\n[tool.fastapi]\nentrypoint = "src_pkg.serve:app"\n',
            "src_pkg/__init__.py": "",
            "src_pkg/serve.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert application.entrypoint.source == "tool-fastapi"
    assert application.entrypoint.as_string == "src_pkg.serve:app"
    assert technology(application, "fastapi").confidence == "high"


def test_entrypoint_hint_validated(tmp_path: Path) -> None:
    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "serve.py": APP_MAIN})
    result = kenbun.scan(tmp_path, entrypoint="serve:app")
    application = app(result)
    assert application.entrypoint.source == "hint"
    assert application.entrypoint.as_string == "serve:app"

    missing = kenbun.scan(tmp_path, entrypoint="nope.missing:app")
    assert "KB503" in codes(missing)


# ── src layout / rule 4 ─────────────────────────────────────────────────────


def test_src_layout_medium_kb111(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "src/api/__init__.py": "",
            "src/api/main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert technology(application, "fastapi").confidence == "medium"
    assert application.entrypoint.as_string == "api.main:app"
    assert application.entrypoint.import_root.endswith("src")
    assert "KB111" in codes(result)


def test_binding_less_main_shadows_real_app(tmp_path: Path) -> None:
    # Production (`fastapi run`) stops at the FIRST existing candidate file.
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": "print('not an app')\n",
            "app/__init__.py": "",
            "app/main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert application.entrypoint.as_string == "app.main:app"
    # Production wouldn't find it because an earlier candidate shadows it.
    assert technology(application, "fastapi").confidence == "medium"
    assert "KB111" in codes(result)


def test_reexport_from_package_init(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "backend/__init__.py": "from .server import app\n",
            "backend/server.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert application.entrypoint.module == "backend"
    assert application.entrypoint.attribute == "app"


# ── failure modes (#976) ────────────────────────────────────────────────────


def test_empty_directory_kb100(tmp_path: Path) -> None:
    (tmp_path / "README.md").write_text("hi")
    result = kenbun.scan(tmp_path)
    assert result.applications == []
    assert "KB100" in codes(result)


def test_syntax_error_kb200_and_kb103(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": "def read_root(\n",
        },
    )
    result = kenbun.scan(tmp_path)
    assert "KB200" in codes(result)
    kb200 = next(d for d in result.diagnostics if d.code == "KB200")
    assert kb200.path == "main.py"
    assert kb200.span is not None and kb200.span.start_line >= 1
    # A declared framework remains an application even without an entrypoint.
    assert "KB103" in codes(result)
    assert len(result.applications) == 1
    application = app(result)
    assert application.entrypoint is None
    assert technology(application, "fastapi").confidence == "medium"


def test_library_project_kb102(tmp_path: Path) -> None:
    make(
        tmp_path,
        {"pyproject.toml": '[project]\nname = "lib"\ndependencies = ["httpx"]\n'},
    )
    result = kenbun.scan(tmp_path)
    assert result.applications == []
    assert "KB102" in codes(result)


def test_convention_only_fastapi_has_primary_technology(tmp_path: Path) -> None:
    make(tmp_path, {"main.py": APP_MAIN})
    application = app(kenbun.scan(tmp_path))
    fastapi = technology(application, "fastapi")
    assert fastapi.role == "primary"
    assert fastapi.confidence == "low"
    assert application.entrypoint.as_string == "main:app"


# ── dependency hygiene (#960) ───────────────────────────────────────────────


def test_poetry_classic_kb306_kb307(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                "[tool.poetry]\n"
                'name = "legacy"\n'
                "[tool.poetry.dependencies]\n"
                'python = "^3.11"\n'
                'fastapi = "^0.115"\n'
            ),
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    all_codes = codes(result)
    assert "KB306" in all_codes  # poetry detected
    assert "KB301" in all_codes  # not on the evident install path
    assert "KB307" in all_codes  # nothing installable declares fastapi
    application = app(result)
    assert technology(application, "fastapi").confidence == "medium"
    assert technology(application, "python").kind == "language"


def test_requirements_only_project(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "requirements.txt": "fastapi[standard]>=0.115\nuvicorn\n",
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert technology(application, "fastapi").confidence == "high"
    python_dependencies = dependencies(application)
    assert python_dependencies.package_manager == "pip"
    assert any(d.name == "fastapi" for d in python_dependencies.declared)


def test_both_manifests_kb300(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "requirements.txt": "flask\n",
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    assert "KB300" in codes(result)


def test_dependency_group_only_kb301(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "x"\ndependencies = []\n'
                "[dependency-groups]\n"
                'extras = ["fastapi"]\n'
            ),
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    assert "KB301" in codes(result)
    assert technology(app(result), "fastapi").confidence == "medium"


def test_python_version_conflict_kb700(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "x"\nrequires-python = ">=3.12"\n'
                'dependencies = ["fastapi"]\n'
            ),
            ".python-version": "3.10\n",
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    assert "KB700" in codes(result)


def test_python_tool_versions_are_inherited_and_preserve_sources(
    tmp_path: Path,
) -> None:
    make(
        tmp_path,
        {
            ".tool-versions": "python 3.13.2 3.12.9\nnodejs 22.14.0\n",
            "apps/api/pyproject.toml": (
                '[project]\nname = "api"\nrequires-python = ">=3.12"\n'
                'dependencies = ["fastapi"]\n'
            ),
            "apps/api/.python-version": "3.13.3\n",
            "apps/api/main.py": APP_MAIN,
        },
    )
    application = app(kenbun.scan(tmp_path), "apps/api")
    assert {(pin.source, pin.value) for pin in application.python.version_pins} == {
        (".tool-versions", "3.12.9"),
        (".tool-versions", "3.13.2"),
        ("apps/api/.python-version", "3.13.3"),
    }
    assert "KB700" not in codes(kenbun.scan(tmp_path))


def test_python_tool_version_conflicts_with_requires_python(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            ".tool-versions": "python 3.11.9\n",
            "pyproject.toml": (
                '[project]\nname = "x"\nrequires-python = ">=3.12"\n'
                'dependencies = ["fastapi"]\n'
            ),
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    diagnostic = next(item for item in result.diagnostics if item.code == "KB700")
    assert ".tool-versions pins 3.11.9" in diagnostic.message


def test_uv_lock_resolved_versions(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": APP_MAIN,
            "uv.lock": (
                'version = 1\n\n[[package]]\nname = "fastapi"\nversion = "0.115.8"\n'
                '\n[[package]]\nname = "starlette"\nversion = "0.45.0"\n'
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    resolved = dependencies(app(result)).resolved
    assert ("fastapi", "0.115.8") in [(r.name, r.version) for r in resolved]


# ── workspaces + origins ────────────────────────────────────────────────────


def workspace_fixture(tmp_path: Path) -> Path:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[tool.uv.workspace]\nmembers = ["apps/*", "packages/*"]\n'
            ),
            "uv.lock": "version = 1\n",
            "apps/api/pyproject.toml": FASTAPI_PYPROJECT,
            "apps/api/app/__init__.py": "",
            "apps/api/app/main.py": APP_MAIN,
            "apps/admin/pyproject.toml": (
                '[project]\nname = "admin"\ndependencies = ["fastapi"]\n'
            ),
            "apps/admin/main.py": APP_MAIN,
            "packages/core/pyproject.toml": (
                '[project]\nname = "core"\ndependencies = ["pydantic"]\n'
            ),
            "packages/core/src/core/__init__.py": "",
        },
    )
    return tmp_path


def test_workspace_from_root(tmp_path: Path) -> None:
    result = kenbun.scan(workspace_fixture(tmp_path))
    assert result.workspace is not None
    assert result.workspace.virtual_root
    assert result.workspace.members == [".", "apps/admin", "apps/api", "packages/core"]
    assert [application.application_dir for application in result.applications] == [
        "apps/admin",
        "apps/api",
    ]


def test_workspace_from_member_preserves_origin(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    result = kenbun.scan(root / "apps" / "api")
    assert result.upload_root == "../.."
    assert result.scan_origin == "apps/api"
    assert [application.application_dir for application in result.applications] == [
        "apps/admin",
        "apps/api",
    ]


def test_workspace_from_library_member_reports_sibling_apps(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    result = kenbun.scan(root / "packages" / "core")
    assert result.scan_origin == "packages/core"
    assert [application.application_dir for application in result.applications] == [
        "apps/admin",
        "apps/api",
    ]


def test_example_and_application_are_deterministically_ordered(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "backend/pyproject.toml": FASTAPI_PYPROJECT,
            "backend/main.py": APP_MAIN,
            "examples/demo/pyproject.toml": FASTAPI_PYPROJECT,
            "examples/demo/main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    assert [application.application_dir for application in result.applications] == [
        "backend",
        "examples/demo",
    ]


def test_application_dir_validation(tmp_path: Path) -> None:
    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "main.py": APP_MAIN})
    assert "KB500" in codes(kenbun.scan(tmp_path, application_dir="nope"))
    assert "KB501" in codes(kenbun.scan(tmp_path, application_dir="../escape"))
    assert "KB501" in codes(kenbun.scan(tmp_path, application_dir="/frontend"))
    assert "KB501" in codes(kenbun.scan(tmp_path, application_dir=r"C:\frontend"))


# ── output contract ─────────────────────────────────────────────────────────


def test_scan_is_deterministic(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    assert kenbun.scan(root).to_json() == kenbun.scan(root).to_json()


def test_to_json_shape(tmp_path: Path) -> None:
    import json

    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "main.py": APP_MAIN})
    data = json.loads(kenbun.scan(tmp_path).to_json())
    assert data["schema_version"] == 1
    assert data["applications"][0]["application_dir"] == "."
    assert data["applications"][0]["entrypoint"]["as_string"] == "main:app"
    assert data["applications"][0]["node"] is None
    assert [item["name"] for item in data["applications"][0]["technologies"]] == [
        "fastapi",
        "python",
    ]
    assert "projects" not in data
    assert "deploy_targets" not in data
    assert "classification" not in data


def test_venv_and_gitignore_excluded(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": APP_MAIN,
            ".venv/lib/site/fastapi/applications.py": (
                "from fastapi import FastAPI\napp = FastAPI()\n"
            ),
            ".venv/pyvenv.cfg": "",
        },
    )
    result = kenbun.scan(tmp_path)
    assert len(result.applications) == 1
    assert app(result).application_dir == "."


# ── SBOM-style manifest fixtures ────────────────────────────────────────────


def test_pipfile_project(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "Pipfile": (
                '[packages]\nfastapi = "==0.115.0"\nuvicorn = "==0.30.0"\n'
                "[dev-packages]\n"
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert technology(application, "fastapi").role == "primary"
    python_dependencies = dependencies(application)
    assert python_dependencies.package_manager == "pipenv"
    assert any(d.name == "fastapi" for d in python_dependencies.declared)
    assert "KB306" in codes(result)


def test_setup_py_string_scan_does_not_create_an_application(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "setup.py": (
                "from setuptools import setup\n"
                "deps = []\n"
                'deps.append("fastapi==0.115.0")\n'
                'setup(name="x", install_requires=deps)\n'
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    # setup.py is string-scanned but never executed or treated as a declaration.
    assert result.applications == []
    assert "KB102" in codes(result)


def test_pdm_backend_detected(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "x"\ndependencies = ["fastapi==0.115.0"]\n'
                '[build-system]\nrequires = ["pdm-backend"]\n'
                'build-backend = "pdm.backend"\n'
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert dependencies(application).package_manager == "pdm"
    assert technology(application, "fastapi").role == "primary"


def test_extra_ignore_files_matches_upload_set(tmp_path: Path) -> None:
    # kenbun must validate against the SAME files that get uploaded: a
    # .fastapicloudignore that hides the entry module means the deployed
    # archive has no app object, so detection must reflect that.
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": APP_MAIN,
            ".fastapicloudignore": "main.py\n",
        },
    )
    default = kenbun.scan(tmp_path)
    assert app(default).entrypoint is not None

    honored = kenbun.scan(tmp_path, extra_ignore_files=[".fastapicloudignore"])
    assert app(honored).entrypoint is None
    assert "KB103" in codes(honored)


def test_workspace_member_hints_are_relative_to_caller_root(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    result = kenbun.scan(
        root / "apps" / "api",
        application_dir=".",
        entrypoint="app.main:app",
    )
    application = app(result, "apps/api")
    assert application.entrypoint.as_string == "app.main:app"
    assert application.entrypoint.source == "hint"
    assert "KB502" not in codes(result)


def test_requirement_groups_preserve_exact_include_group_keys(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "x"\ndependencies = []\n'
                "[dependency-groups]\n"
                'Web_API = ["fastapi"]\n'
                'production = [{include-group = "Web_API"}]\n'
            ),
            "main.py": APP_MAIN,
        },
    )
    declared = dependencies(app(kenbun.scan(tmp_path))).declared
    assert [item.name for item in declared].count("fastapi") == 2


def test_requirements_include_is_not_double_counted(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "requirements.txt": "-r requirements/base.txt\nuvicorn\n",
            "requirements/base.txt": "fastapi\n",
            "main.py": APP_MAIN,
        },
    )
    declared = dependencies(app(kenbun.scan(tmp_path))).declared
    assert [item.name for item in declared] == ["fastapi", "uvicorn"]


def test_bare_requirement_urls_do_not_fake_frameworks(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "requirements.txt": (
                "https://example.invalid/fastapi.whl\n"
                "git+https://example.invalid/fastapi.git\n"
                "httpx @ https://example.invalid/httpx.whl\n"
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    assert result.applications == []


def test_application_dir_name_tokens_do_not_make_main_requirements_dev(
    tmp_path: Path,
) -> None:
    root = tmp_path / "devportal"
    make(root, {"requirements.txt": "fastapi\n", "main.py": APP_MAIN})
    result = kenbun.scan(root)
    assert "KB301" not in codes(result)
    assert "KB307" not in codes(result)


def test_rule4_package_discovery_under_subdirectory(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "api/pyproject.toml": FASTAPI_PYPROJECT,
            "api/service/__init__.py": "",
            "api/service/main.py": APP_MAIN,
        },
    )
    application = app(kenbun.scan(tmp_path), "api")
    assert application.entrypoint.as_string == "service.main:app"


def test_entrypoint_accepts_conditional_binding_and_rejects_invalid_modules(
    tmp_path: Path,
) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": "if True:\n    app = object()\n",
        },
    )
    accepted = kenbun.scan(tmp_path, entrypoint="main:app")
    assert app(accepted).entrypoint.as_string == "main:app"
    assert "KB504" not in codes(accepted)

    for invalid in ["bad-path:app", "bad/path:app", "main:bad-name"]:
        assert "KB503" in codes(kenbun.scan(tmp_path, entrypoint=invalid))


def test_absolute_self_reexport_is_resolved(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "backend/__init__.py": "from backend.server import app\n",
            "backend/server.py": APP_MAIN,
        },
    )
    application = app(kenbun.scan(tmp_path))
    assert application.entrypoint.as_string == "backend:app"


def test_pep723_inline_script_dependencies(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "main.py": (
                "# /// script\n"
                '# requires-python = ">=3.12"\n'
                '# dependencies = ["fastapi", "uvicorn"]\n'
                "# ///\n" + APP_MAIN
            ),
        },
    )
    application = app(kenbun.scan(tmp_path))
    assert {item.name for item in dependencies(application).declared} == {
        "fastapi",
        "uvicorn",
    }
    assert application.python.requires_python == ">=3.12"


def test_legacy_pdm_dependencies_are_parsed(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[tool.pdm]\ndependencies = ["fastapi", "uvicorn"]\n'
                '[tool.pdm.dev-dependencies]\ntest = ["pytest"]\n'
            ),
            "main.py": APP_MAIN,
        },
    )
    application = app(kenbun.scan(tmp_path))
    assert {item.name for item in dependencies(application).declared} == {
        "fastapi",
        "pytest",
        "uvicorn",
    }


def test_ignore_file_is_not_an_implicit_scan_exclusion(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            ".ignore": "main.py\n",
            "requirements.txt": "fastapi\n",
            "main.py": APP_MAIN,
        },
    )
    assert app(kenbun.scan(tmp_path)).entrypoint.as_string == "main:app"


def test_invalid_roots_have_a_filesystem_diagnostic(tmp_path: Path) -> None:
    missing = kenbun.scan(tmp_path / "missing")
    assert "KB800" in codes(missing)

    regular_file = tmp_path / "file.txt"
    regular_file.write_text("not a directory")
    assert "KB800" in codes(kenbun.scan(regular_file))


def test_unavailable_pyproject_is_diagnostic(tmp_path: Path) -> None:
    (tmp_path / "pyproject.toml").write_bytes(b"x" * (2 * 1024 * 1024 + 1))
    result = kenbun.scan(tmp_path)
    assert "KB801" in codes(result)


def test_distinct_diagnostic_messages_at_same_location_are_preserved(
    tmp_path: Path,
) -> None:
    make(
        tmp_path,
        {
            "package.json": '{"workspaces": ["apps/*", "packages/*"]}',
        },
    )
    kb402 = [item for item in kenbun.scan(tmp_path).diagnostics if item.code == "KB402"]
    assert len(kb402) == 2


def test_scan_budget_and_external_symlink_are_bounded(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "requirements.txt": "fastapi\n",
            "main.py": APP_MAIN,
            "extra.py": "pass\n",
        },
    )
    assert "KB802" in codes(kenbun.scan(tmp_path, max_files=1))

    outside = tmp_path.parent / f"{tmp_path.name}-outside.txt"
    outside.write_text("secret-framework-name\n")
    try:
        (tmp_path / "outside.txt").symlink_to(outside)
    except OSError:
        pytest.skip("symlinks unavailable")
    payload = kenbun.scan(tmp_path, follow_symlinks=True).to_json()
    assert "secret-framework-name" not in payload


@pytest.mark.parametrize("dependency", ["django", "flask"])
def test_identity_frameworks_have_coverage(tmp_path: Path, dependency: str) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                f'[project]\nname = "x"\ndependencies = ["{dependency}"]\n'
            ),
        },
    )
    assert technology(app(kenbun.scan(tmp_path)), dependency).role == "primary"


def test_manage_py_detects_django_without_dependency(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "manage.py": (
                "import os\n"
                'os.environ.setdefault("DJANGO_SETTINGS_MODULE", "demo.settings")\n'
            ),
        },
    )
    assert technology(app(kenbun.scan(tmp_path)), "django").role == "primary"


def test_remaining_diagnostic_codes_have_regression_coverage(tmp_path: Path) -> None:
    router = tmp_path / "router"
    make(
        router,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": "from fastapi import APIRouter\nrouter = APIRouter()\n",
        },
    )
    assert "KB104" in codes(kenbun.scan(router))

    malformed = tmp_path / "malformed"
    make(malformed, {"pyproject.toml": "[project\n"})
    assert "KB201" in codes(kenbun.scan(malformed))

    tableless = tmp_path / "tableless"
    make(tableless, {"pyproject.toml": '[tool.demo]\nname = "x"\n'})
    assert "KB202" in codes(kenbun.scan(tableless))

    locks = tmp_path / "locks"
    make(
        locks,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": APP_MAIN,
            "uv.lock": "version = 1\n",
            "pdm.lock": "",
        },
    )
    assert "KB305" in codes(kenbun.scan(locks))

    workspace = tmp_path / "workspace"
    make(
        workspace,
        {
            "pyproject.toml": '[tool.uv.workspace]\nmembers = ["apps/*"]\n',
            "apps/missing/README.md": "missing manifest\n",
            "apps/nested/pyproject.toml": (
                '[tool.uv.workspace]\nmembers = ["packages/*"]\n'
            ),
        },
    )
    workspace_codes = codes(kenbun.scan(workspace))
    assert "KB400" in workspace_codes
    assert "KB401" in workspace_codes

    hinted = tmp_path / "hinted"
    make(
        hinted,
        {
            "pyproject.toml": FASTAPI_PYPROJECT,
            "main.py": "app = object()\n",
        },
    )
    assert "KB505" in codes(kenbun.scan(hinted, entrypoint="main:app"))
