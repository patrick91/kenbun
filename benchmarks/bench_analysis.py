from collections.abc import Mapping

import kenbun

FASTAPI_APP = b"from fastapi import FastAPI\napp = FastAPI()\n"
Repository = tuple[list[kenbun.FileEntry], dict[str, bytes]]


def repository(
    contents: Mapping[str, bytes],
) -> Repository:
    ordered_contents = dict(sorted(contents.items()))
    files = [
        kenbun.FileEntry(path=path, size=len(source))
        for path, source in ordered_contents.items()
    ]
    return files, ordered_contents


def framework_manifest(name: str, dependency: str) -> bytes:
    return f'[project]\nname = "{name}"\ndependencies = ["{dependency}"]\n'.encode()


def fastapi_manifest(name: str) -> bytes:
    return framework_manifest(name, "fastapi")


SINGLE_FASTAPI = repository(
    {
        "pyproject.toml": fastapi_manifest("single"),
        "main.py": FASTAPI_APP,
    }
)

FASTAPI_MONOREPO = repository(
    {
        path: source
        for index in range(64)
        for path, source in [
            (
                f"apps/service_{index:02}/pyproject.toml",
                fastapi_manifest(f"service-{index:02}"),
            ),
            (f"apps/service_{index:02}/main.py", FASTAPI_APP),
        ]
    }
)

IDENTITY_FRAMEWORK_MONOREPO = repository(
    {
        f"services/service_{index:03}/pyproject.toml": framework_manifest(
            f"service-{index:03}",
            "django" if index % 2 == 0 else "flask",
        )
        for index in range(128)
    }
)

FASTAPI_FALLBACK = repository(
    {
        "pyproject.toml": fastapi_manifest("fallback"),
        **{
            f"src/package_{index:02}/__init__.py": b"VALUE = 1\n" for index in range(32)
        },
        "src/package_zz/__init__.py": b"",
        "src/package_zz/main.py": FASTAPI_APP,
    }
)


def test_analyze_single_fastapi_application(benchmark) -> None:
    files, contents = SINGLE_FASTAPI

    result = benchmark(kenbun.analyze, files, contents, ecosystems={"python"})

    assert len(result.applications) == 1
    assert result.applications[0].entrypoint.as_string == "main:app"


def test_analyze_fastapi_monorepo(benchmark) -> None:
    files, contents = FASTAPI_MONOREPO

    result = benchmark(kenbun.analyze, files, contents, ecosystems={"python"})

    assert len(result.applications) == 64
    assert result.applications[-1].application_dir == "apps/service_63"


def test_analyze_identity_framework_monorepo(benchmark) -> None:
    files, contents = IDENTITY_FRAMEWORK_MONOREPO

    result = benchmark(kenbun.analyze, files, contents, ecosystems={"python"})

    assert len(result.applications) == 128
    assert result.applications[-1].application_dir == "services/service_127"


def test_analyze_fastapi_source_fallback(benchmark) -> None:
    files, contents = FASTAPI_FALLBACK

    result = benchmark(kenbun.analyze, files, contents, ecosystems={"python"})

    assert len(result.applications) == 1
    assert result.applications[0].entrypoint.as_string == "package_zz.main:app"


def test_analyze_fastapi_service_fixture(
    benchmark,
    fastapi_service_fixture: Repository,
) -> None:
    files, contents = fastapi_service_fixture

    result = benchmark(kenbun.analyze, files, contents)

    assert len(result.applications) == 1
    assert result.applications[0].name == "fixture-api"
    assert result.applications[0].entrypoint.as_string == "fixture_api.main:app"


def test_analyze_complex_workspace_fixture(
    benchmark,
    complex_workspace_fixture: Repository,
) -> None:
    files, contents = complex_workspace_fixture

    result = benchmark(kenbun.analyze, files, contents)

    assert result.workspace.kind == "mixed"
    assert result.workspace.members == [
        ".",
        "apps/dashboard",
        "apps/docs",
        "apps/web",
        "packages/shared",
        "packages/ui",
        "services/admin",
        "services/api",
    ]
    assert [application.application_dir for application in result.applications] == [
        "apps/dashboard",
        "apps/docs",
        "apps/web",
        "services/admin",
        "services/api",
        "services/legacy",
    ]
    api = next(
        application
        for application in result.applications
        if application.application_dir == "services/api"
    )
    assert {technology.name for technology in api.technologies} >= {
        "cross-inertia",
        "fastapi",
        "vite",
    }
