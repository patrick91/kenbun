# kenbun

`kenbun` (見聞 — "seeing and hearing") is a Rust-backed Python library that
statically discovers applications in a repository. It reports application
boundaries, languages, frameworks, integrations, dependency manifests, build
scripts, entrypoints, and diagnostics without installing dependencies or
executing repository code.

Kenbun reports facts rather than deployment policy. A CLI or platform can use
the result to present application choices and derive commands, but Kenbun does
not select an application, decide whether it is deployable, or construct a
runtime command.

## Usage

```python
from pathlib import Path

import kenbun

result = kenbun.scan(
    Path("."),
    application_dir=None,  # optional repository-relative hint
    entrypoint=None,       # optional FastAPI "module:attribute" hint
)

for application in result.applications:
    print(application.application_dir)
    print(
        [
            (technology.name, technology.kind, technology.role)
            for technology in application.technologies
        ]
    )

print(result.to_json())
```

`scan()` currently accepts a real directory only. It walks the repository,
honors built-in and caller-provided ignore rules, discovers supported
workspaces, and returns `ScanResult` with `schema_version == 1`. Results are
deterministically ordered and available as typed PyO3 objects or canonical
JSON.

## Supported detection

Python applications:

- FastAPI, including static entrypoint resolution and dependency diagnostics.
- Django and Flask identity detection. Kenbun does not infer their entrypoints
  yet.

Node applications:

- Next.js, Astro, Nuxt, SvelteKit, TanStack Start, React Router Framework Mode,
  SolidStart, and legacy Remix.
- Standalone Vite applications, using a deliberately strict rule: the same
  directory must directly depend on Vite, define an explicit `build` script
  that directly invokes `vite build`, and contain `index.html`.
- React, Vue, Svelte, and Solid as supporting UI-framework facts.

Vite can also be supporting build tooling for another application. In
particular, a FastAPI application using Vite for frontend assets remains one
FastAPI application rather than becoming a second Vite application.

Cross Inertia is reported as the normalized `cross-inertia` integration when its
same-directory Python and Node evidence agree. Generic Inertia integrations
are deferred.

Kenbun understands uv, npm, pnpm, Yarn, and Bun workspaces, including roots
that declare both Python and Node workspace metadata. Technology and build-tool
evidence attaches only within one application directory; a nested package is
kept separate and must independently qualify as an application.

## Output model

- `ScanResult` contains the scan paths, optional `Workspace`, ordered
  `applications`, and aggregate diagnostics.
- `Application` contains `technologies`, optional entrypoint, one or more
  ecosystem-specific `DependencySet` values, explicit `build_scripts`, Python
  and Node runtime metadata, evidence, and local diagnostics.
- `Technology` has a normalized name, kind (`language`, `framework`,
  `ui-framework`, `integration`, or `build-tool`), role (`primary` or
  `supporting`), confidence, and evidence.
- `BuildScript` records the explicit `build` script as data: the raw command,
  optional safely parsed argv, optional inferred package manager, and source.

See the [v1 specification](docs/spec.md) for the normative model and detection
rules.

## External fixture corpus

The normal test suite is self-contained and does not require network access.
For manual acceptance testing, the external runner downloads immutable GitHub
archives listed in `tests/external_fixtures.json`, scans them without installing
or executing their code, and compares a stable projection:

```bash
uv run python scripts/check_external_fixtures.py
uv run python scripts/check_external_fixtures.py --fixture fastapi-basic
uv run python scripts/check_external_fixtures.py --offline
```

Every fixture is pinned to a full commit SHA; the runner never follows a
default branch. Archives are cached under `target/github-fixtures`.

## Development

Build the extension and run the tests with:

```bash
uv run maturin develop --uv
uv run pytest
cargo test --all-targets --all-features
cargo deny check
```
