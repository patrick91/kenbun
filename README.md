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

For remote repositories, pass an inventory first and feed requested contents
back into the stateless analyzer:

```python
files = [
    kenbun.FileEntry("pyproject.toml", 128, "git-blob-sha"),
    kenbun.FileEntry("app.py", 512, "another-blob-sha"),
]
contents: dict[str, bytes | None] = {}

while True:
    result = kenbun.analyze(
        files,
        contents,
        inventory_complete=True,
        hints={"script_patterns": ["main.py", "app.py", "api.py"]},
    )
    if result.status == "complete":
        break
    for wanted in result.want_files:
        contents[wanted.path] = fetch_blob(wanted.blob_sha)  # or None
```

`scan()` walks a real directory. `analyze()` is sans-I/O and incrementally
requests only the contents it needs. Both return schema v2 `ScanResult`
objects with deterministic ordering and canonical JSON.

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

- `ScanResult` contains protocol status/completeness, ordered `want_files`,
  scan paths, optional `Workspace`, ordered `applications`, and diagnostics.
- `Application` contains `technologies`, optional entrypoint, one or more
  ecosystem-specific `DependencySet` values, explicit `build_scripts`, Python
  and Node runtime metadata, evidence, and local diagnostics.
- `Technology` has a normalized name, kind (`language`, `framework`,
  `ui-framework`, `integration`, or `build-tool`), role (`primary` or
  `supporting`), confidence, and evidence.
- `BuildScript` records the explicit `build` script as data: the raw command,
  optional safely parsed argv, optional inferred package manager, and source.

See the [v2 specification](docs/spec.md) for the normative model and detection
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
