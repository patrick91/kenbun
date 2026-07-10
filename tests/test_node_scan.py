import json
from pathlib import Path

import pytest

import kenbun


def make(root: Path, files: dict[str, str]) -> None:
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


def package(
    *,
    name: str = "fixture",
    dependencies: dict[str, str] | None = None,
    dev_dependencies: dict[str, str] | None = None,
    scripts: dict[str, str] | None = None,
    package_manager: str | None = "npm@12.0.0",
    workspaces: list[str] | None = None,
) -> str:
    data: dict[str, object] = {
        "name": name,
        "private": True,
        "dependencies": dependencies or {},
        "devDependencies": dev_dependencies or {},
        "scripts": scripts or {},
    }
    if package_manager is not None:
        data["packageManager"] = package_manager
    if workspaces is not None:
        data["workspaces"] = workspaces
    return json.dumps(data)


def app(result: kenbun.ScanResult, application_dir: str = ".") -> kenbun.Application:
    return next(
        application
        for application in result.applications
        if application.application_dir == application_dir
    )


def technology(application: kenbun.Application, name: str) -> kenbun.Technology:
    return next(item for item in application.technologies if item.name == name)


@pytest.mark.parametrize(
    ("dependency", "expected", "extra_dependencies", "files", "build"),
    [
        ("next", "nextjs", {"react": "19"}, {}, "next build"),
        (
            "astro",
            "astro",
            {},
            {"astro.config.mjs": "export default {}"},
            "astro build",
        ),
        (
            "nuxt",
            "nuxt",
            {"vue": "3"},
            {"nuxt.config.ts": "export default {}"},
            "nuxt build",
        ),
        (
            "@sveltejs/kit",
            "sveltekit",
            {"svelte": "5", "vite": "8"},
            {"svelte.config.js": "export default {}"},
            "vite build",
        ),
        (
            "@tanstack/react-start",
            "tanstack-start",
            {"react": "19", "vite": "8"},
            {"vite.config.ts": "export default {}"},
            "vite build",
        ),
        (
            "@react-router/dev",
            "react-router",
            {"react": "19", "vite": "8"},
            {"react-router.config.ts": "export default {}"},
            "react-router build",
        ),
        ("@solidjs/start", "solidstart", {"solid-js": "1"}, {}, "vinxi build"),
        (
            "@remix-run/dev",
            "remix",
            {"react": "19", "vite": "8"},
            {},
            "remix vite:build",
        ),
    ],
)
def test_framework_identity(
    tmp_path: Path,
    dependency: str,
    expected: str,
    extra_dependencies: dict[str, str],
    files: dict[str, str],
    build: str,
) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                dependencies={dependency: "1", **extra_dependencies},
                dev_dependencies={"typescript": "6"},
                scripts={"build": build},
            ),
            "tsconfig.json": "{}",
            "src/main.ts": "export {};",
            **files,
        },
    )

    result = kenbun.scan(tmp_path)
    application = app(result)
    primary = technology(application, expected)
    assert primary.kind == "framework"
    assert primary.role == "primary"
    assert technology(application, "typescript").kind == "language"
    assert application.build_scripts[0].command == build
    assert application.build_scripts[0].package_manager == "npm"


def test_react_router_library_mode_is_not_an_application(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(dependencies={"react-router": "7", "react": "19"}),
            "src/index.js": "export {};",
        },
    )
    assert kenbun.scan(tmp_path).applications == []


def test_strict_vite_application_and_safe_build_argv(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                dependencies={"react": "19"},
                dev_dependencies={"vite": "8"},
                scripts={"build": "vite build"},
            ),
            "index.html": "<!doctype html>",
            "src/main.jsx": "export {};",
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    vite = technology(application, "vite")
    assert vite.kind == "build-tool"
    assert vite.role == "primary"
    assert technology(application, "javascript").role == "supporting"
    assert application.build_scripts[0].argv == ["vite", "build"]

    payload = json.loads(result.to_json())
    assert payload["applications"][0]["dependencies"][0]["ecosystem"] == "node"
    assert payload["applications"][0]["build_scripts"][0]["command"] == "vite build"


def test_vite_library_without_index_is_not_an_application(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                dev_dependencies={"vite": "8"},
                scripts={"build": "vite build"},
            ),
            "vite.config.js": "export default {};",
            "src/index.js": "export {};",
        },
    )
    assert kenbun.scan(tmp_path).applications == []


def test_cross_inertia_merges_same_root_node_facts(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "cross-app"\n'
                'dependencies = ["fastapi", "cross-inertia"]\n'
            ),
            "main.py": "from fastapi import FastAPI\napp = FastAPI()\n",
            "package.json": package(
                dependencies={"@inertiajs/react": "3", "react": "19"},
                dev_dependencies={"typescript": "6", "vite": "8"},
                scripts={"build": "tsc && vite build"},
                package_manager="bun@1.3.14",
            ),
            "tsconfig.json": "{}",
            "vite.config.js": "export default {};",
            "frontend/app.tsx": "export {};",
        },
    )
    application = app(kenbun.scan(tmp_path))
    assert technology(application, "fastapi").role == "primary"
    assert technology(application, "vite").role == "supporting"
    assert technology(application, "cross-inertia").kind == "integration"
    inertia_paths = {
        evidence.path for evidence in technology(application, "cross-inertia").evidence
    }
    assert inertia_paths == {"package.json", "pyproject.toml"}
    assert {item.ecosystem for item in application.dependencies} == {"python", "node"}
    assert application.entrypoint.as_string == "main:app"
    assert application.build_scripts[0].argv is None


def test_same_root_primary_frameworks_share_one_application(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": '[project]\nname = "mixed"\ndependencies = ["fastapi"]\n',
            "main.py": "from fastapi import FastAPI\napp = FastAPI()\n",
            "package.json": package(dependencies={"next": "16", "react": "19"}),
            "app/page.jsx": "export default function Page() {}",
        },
    )
    result = kenbun.scan(tmp_path)
    assert len(result.applications) == 1
    primary = {
        item.name
        for item in result.applications[0].technologies
        if item.role == "primary"
    }
    assert primary == {"fastapi", "nextjs"}
    assert "KB101" in [diagnostic.code for diagnostic in result.diagnostics]


def test_node_primary_preserves_same_root_python_library_facts(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "worker"\ndependencies = ["httpx"]\n'
            ),
            "worker.py": "import httpx\n",
            "package.json": package(dependencies={"astro": "6"}),
            "astro.config.mjs": "export default {};",
        },
    )
    application = app(kenbun.scan(tmp_path))
    assert application.name == "fixture"
    assert technology(application, "astro").role == "primary"
    assert technology(application, "python").role == "supporting"
    assert {item.ecosystem for item in application.dependencies} == {"python", "node"}
    assert application.python is not None


def test_nested_node_library_does_not_attach_to_parent(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": '[project]\nname = "api"\ndependencies = ["fastapi"]\n',
            "main.py": "from fastapi import FastAPI\napp = FastAPI()\n",
            "frontend/package.json": package(dependencies={"react": "19"}),
            "frontend/index.jsx": "export {};",
        },
    )
    result = kenbun.scan(tmp_path)
    assert [item.application_dir for item in result.applications] == ["."]
    names = {item.name for item in result.applications[0].technologies}
    assert "react" not in names


def test_node_workspace_is_found_from_member(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                name="workspace",
                package_manager="pnpm@11.0.0",
            ),
            "pnpm-workspace.yaml": "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
            "apps/site/package.json": package(
                name="site",
                dependencies={"astro": "6"},
                dev_dependencies={"typescript": "6"},
                scripts={"build": "astro build"},
                package_manager=None,
            ),
            "apps/site/astro.config.mjs": "export default {};",
            "apps/site/tsconfig.json": "{}",
            "apps/site/src/pages/index.astro": "<h1>Hello</h1>",
            "packages/lib/package.json": package(
                name="lib",
                package_manager=None,
            ),
        },
    )
    result = kenbun.scan(tmp_path / "apps" / "site" / "src" / "pages")
    assert result.upload_root == "../../../.."
    assert result.scan_origin == "apps/site/src/pages"
    assert result.workspace.kind == "pnpm"
    assert result.workspace.members == [".", "apps/site", "packages/lib"]
    assert app(result, "apps/site").dependencies[0].package_manager == "pnpm"


def test_workspace_glob_does_not_claim_non_package_directory(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                name="workspace",
                workspaces=["apps/*"],
            ),
            "apps/web/package.json": package(dependencies={"next": "16"}),
            "apps/docs/content/README.md": "Not a package",
        },
    )
    result = kenbun.scan(tmp_path / "apps" / "docs" / "content")
    assert result.upload_root == "."
    assert result.workspace is None
    assert result.applications == []


def test_mixed_uv_and_node_workspace_discovers_node_member(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": '[tool.uv.workspace]\nmembers = ["backend/*"]\n',
            "backend/api/pyproject.toml": (
                '[project]\nname = "api"\ndependencies = ["fastapi"]\n'
            ),
            "backend/api/main.py": "from fastapi import FastAPI\napp = FastAPI()\n",
            "package.json": package(
                name="mixed-workspace",
                package_manager="pnpm@11.0.0",
            ),
            "pnpm-workspace.yaml": "packages:\n  - 'apps/*'\n",
            "apps/site/package.json": package(
                name="site",
                dependencies={"astro": "6"},
                package_manager=None,
            ),
            "apps/site/astro.config.mjs": "export default {};",
            "apps/site/src/pages/index.astro": "<h1>Hello</h1>",
        },
    )
    result = kenbun.scan(tmp_path / "apps" / "site" / "src")
    assert result.upload_root == "../../.."
    assert result.workspace.kind == "mixed"
    assert result.workspace.members == [".", "apps/site", "backend/api"]
    assert [item.application_dir for item in result.applications] == [
        "apps/site",
        "backend/api",
    ]


def test_workspace_virtual_root_follows_root_application(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "api"\ndependencies = ["fastapi"]\n\n'
                '[tool.uv.workspace]\nmembers = ["packages/*"]\n'
            ),
            "main.py": "from fastapi import FastAPI\napp = FastAPI()\n",
            "packages/lib/pyproject.toml": (
                '[project]\nname = "lib"\ndependencies = []\n'
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    assert result.workspace.virtual_root is False


def test_workspace_root_library_is_virtual(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "pyproject.toml": (
                '[project]\nname = "root-lib"\ndependencies = []\n\n'
                '[tool.uv.workspace]\nmembers = ["packages/*"]\n'
            ),
            "packages/lib/pyproject.toml": (
                '[project]\nname = "lib"\ndependencies = []\n'
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    assert result.workspace.virtual_root is True


@pytest.mark.parametrize("manager", ["npm", "yarn", "bun"])
def test_node_workspace_manager_kinds(tmp_path: Path, manager: str) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                name="workspace",
                package_manager=f"{manager}@1.0.0",
                workspaces=["apps/*"],
            ),
            "apps/web/package.json": package(
                dependencies={"next": "16"},
                package_manager=None,
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    assert result.workspace.kind == manager
    assert app(result, "apps/web").dependencies[0].package_manager == manager


def test_node_workspace_kind_without_manager_evidence(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                name="workspace",
                package_manager=None,
                workspaces=["apps/*"],
            ),
            "apps/web/package.json": package(
                dependencies={"next": "16"},
                package_manager=None,
            ),
        },
    )
    result = kenbun.scan(tmp_path)
    assert result.workspace.kind == "node"
    assert app(result, "apps/web").dependencies[0].package_manager is None


def test_ambiguous_manager_does_not_default_to_npm(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(
                dependencies={"next": "16", "react": "19"},
                scripts={"build": "next build"},
                package_manager=None,
            ),
            "package-lock.json": "{}",
            "yarn.lock": "",
            "app/page.jsx": "export default function Page() {}",
        },
    )
    result = kenbun.scan(tmp_path)
    application = app(result)
    assert application.dependencies[0].package_manager is None
    assert application.build_scripts[0].package_manager is None
    assert "KB308" in [diagnostic.code for diagnostic in result.diagnostics]


def test_library_manager_ambiguity_is_still_diagnostic(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "package.json": package(package_manager=None),
            "package-lock.json": "{}",
            "yarn.lock": "",
        },
    )
    result = kenbun.scan(tmp_path)
    assert result.applications == []
    assert "KB308" in [diagnostic.code for diagnostic in result.diagnostics]


def test_application_dir_accepts_node_application(tmp_path: Path) -> None:
    make(
        tmp_path,
        {
            "frontend/package.json": package(dependencies={"next": "16"}),
            "frontend/app/page.js": "export default function Page() {}",
        },
    )
    result = kenbun.scan(tmp_path, application_dir="frontend")
    assert [item.application_dir for item in result.applications] == ["frontend"]
    assert "KB502" not in [diagnostic.code for diagnostic in result.diagnostics]


def test_malformed_package_json_is_diagnostic_not_exception(tmp_path: Path) -> None:
    make(tmp_path, {"package.json": "{broken"})
    result = kenbun.scan(tmp_path)
    assert result.applications == []
    assert "KB203" in [diagnostic.code for diagnostic in result.diagnostics]
