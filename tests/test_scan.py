"""Fixture tests for kenbun.scan — spec §15 golden-corpus subset."""

from pathlib import Path

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

    assert len(result.deploy_targets) == 1
    target = result.deploy_targets[0]
    assert target.project_path == "."
    assert target.confidence == "high"
    assert target.recommended
    assert target.entrypoint is not None
    assert target.entrypoint.as_string == "app.main:app"
    assert target.entrypoint.source == "inferred"
    assert not target.entrypoint.is_factory
    assert result.classification.uses_fastapi == "yes"
    assert result.classification.python == "yes"


def test_flat_main_py(tmp_path: Path) -> None:
    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "main.py": APP_MAIN})
    result = kenbun.scan(tmp_path)
    assert result.deploy_targets[0].entrypoint.as_string == "main:app"


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
    target = result.deploy_targets[0]
    assert target.entrypoint.as_string == "main:app"
    assert not target.entrypoint.is_factory
    assert target.confidence == "high"


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
    target = result.deploy_targets[0]
    assert target.entrypoint.is_factory
    assert target.entrypoint.attribute == "create_app"
    assert target.confidence == "medium"
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
    assert result.deploy_targets[0].entrypoint.attribute == "app"


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
    target = result.deploy_targets[0]
    assert target.entrypoint.source == "tool-fastapi"
    assert target.entrypoint.as_string == "src_pkg.serve:app"
    assert target.confidence == "high"


def test_entrypoint_hint_validated(tmp_path: Path) -> None:
    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "serve.py": APP_MAIN})
    result = kenbun.scan(tmp_path, entrypoint="serve:app")
    target = result.deploy_targets[0]
    assert target.entrypoint.source == "hint"
    assert target.entrypoint.as_string == "serve:app"

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
    target = result.deploy_targets[0]
    assert target.confidence == "medium"
    assert target.entrypoint.as_string == "api.main:app"
    assert target.entrypoint.import_root.endswith("src")
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
    target = result.deploy_targets[0]
    assert target.entrypoint.as_string == "app.main:app"
    assert target.confidence == "medium"  # production wouldn't find it
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
    target = result.deploy_targets[0]
    assert target.entrypoint.module == "backend"
    assert target.entrypoint.attribute == "app"


# ── failure modes (#976) ────────────────────────────────────────────────────


def test_empty_directory_kb100(tmp_path: Path) -> None:
    (tmp_path / "README.md").write_text("hi")
    result = kenbun.scan(tmp_path)
    assert result.deploy_targets == []
    assert "KB100" in codes(result)
    assert result.classification.python == "no"
    assert result.classification.uses_fastapi == "no"


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
    # framework declared but no app object: placeholder target + KB103
    assert "KB103" in codes(result)
    assert len(result.deploy_targets) == 1
    assert result.deploy_targets[0].entrypoint is None


def test_library_project_kb102(tmp_path: Path) -> None:
    make(
        tmp_path,
        {"pyproject.toml": '[project]\nname = "lib"\ndependencies = ["httpx"]\n'},
    )
    result = kenbun.scan(tmp_path)
    assert result.deploy_targets == []
    assert "KB102" in codes(result)
    assert result.classification.python == "yes"
    assert result.classification.uses_fastapi == "no"


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
    assert result.classification.uses_fastapi == "yes"
    # target exists but is capped
    assert result.deploy_targets[0].confidence == "medium"


def test_requirements_only_project(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "requirements.txt": "fastapi[standard]>=0.115\nuvicorn\n",
            "main.py": APP_MAIN,
        },
    )
    result = kenbun.scan(tmp_path)
    target = result.deploy_targets[0]
    assert target.confidence == "high"
    project = result.projects[0]
    assert project.dependencies.package_manager == "pip"
    assert any(d.name == "fastapi" for d in project.dependencies.declared)


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
    assert result.deploy_targets[0].confidence == "medium"


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
    resolved = result.projects[0].dependencies.resolved
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
    assert [t.project_path for t in result.deploy_targets] == ["apps/admin", "apps/api"]
    # tie at high confidence, no affinity → KB110, deterministic recommendation
    assert "KB110" in codes(result)
    assert result.deploy_targets[0].recommended


def test_workspace_from_member_affinity(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    result = kenbun.scan(root / "apps" / "api")
    assert result.upload_root == "../.."
    assert result.scan_origin == "apps/api"
    recommended = [t for t in result.deploy_targets if t.recommended]
    assert recommended[0].project_path == "apps/api"
    assert "KB110" not in codes(result)  # location disambiguates


def test_workspace_from_library_member_kb115(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    result = kenbun.scan(root / "packages" / "core")
    assert "KB115" in codes(result)
    assert result.deploy_targets  # sibling apps still reported


def test_example_never_recommended(tmp_path: Path) -> None:
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
    recommended = [t for t in result.deploy_targets if t.recommended]
    assert recommended[0].project_path == "backend"
    assert "KB110" not in codes(result)  # an example can't create ambiguity


def test_target_dir_validation(tmp_path: Path) -> None:
    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "main.py": APP_MAIN})
    assert "KB500" in codes(kenbun.scan(tmp_path, target_dir="nope"))
    assert "KB501" in codes(kenbun.scan(tmp_path, target_dir="../escape"))


# ── output contract ─────────────────────────────────────────────────────────


def test_scan_is_deterministic(tmp_path: Path) -> None:
    root = workspace_fixture(tmp_path)
    assert kenbun.scan(root).to_json() == kenbun.scan(root).to_json()


def test_to_json_shape(tmp_path: Path) -> None:
    import json

    make(tmp_path, {"pyproject.toml": FASTAPI_PYPROJECT, "main.py": APP_MAIN})
    data = json.loads(kenbun.scan(tmp_path).to_json())
    assert data["schema_version"] == 0
    assert data["deploy_targets"][0]["entrypoint"]["as_string"] == "main:app"
    assert data["classification"]["uses_fastapi"] == "yes"


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
    assert len(result.deploy_targets) == 1
    assert result.deploy_targets[0].project_path == "."


# ── SBOM-style classification fixtures ──────────────────────────────────────


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
    assert result.classification.uses_fastapi == "yes"
    project = result.projects[0]
    assert project.dependencies.package_manager == "pipenv"
    assert any(d.name == "fastapi" for d in project.dependencies.declared)
    assert "KB306" in codes(result)


def test_setup_py_string_scan_is_likely(tmp_path: Path) -> None:
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
    # string scan only: never "yes", never a declared dep — spec §13
    assert result.classification.uses_fastapi == "likely"
    assert result.classification.python == "yes"
    assert all(
        d.name != "fastapi"
        for p in result.projects
        if p.dependencies
        for d in p.dependencies.declared
    )


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
    assert result.projects[0].dependencies.package_manager == "pdm"
    assert result.classification.uses_fastapi == "yes"


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
    assert default.deploy_targets[0].entrypoint is not None

    honored = kenbun.scan(tmp_path, extra_ignore_files=[".fastapicloudignore"])
    assert honored.deploy_targets[0].entrypoint is None
    assert "KB103" in codes(honored)
