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
    ecosystems={"python", "node"},  # optional; defaults to both
    application_dir=None,  # optional repository-relative hint
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

For remote repositories, pass the repository-relative paths and feed requested
contents into a stateful analysis:

```python
files: list[kenbun.FileEntry] = [
    kenbun.FileEntry(path="pyproject.toml", size=128),
    kenbun.FileEntry(path="app.py", size=512),
]
analysis = kenbun.remote_analysis(
    files,
    inventory_complete=True,
    hints={"script_patterns": ["main.py", "app.py", "api.py"]},
    max_files=256,
    max_file_bytes=256 * 1024,
)

while file_requests := analysis.file_requests:
    contents = fetch_files(file_requests)
    analysis.update(contents)

result = analysis.result
```

Pass `ecosystems={"python"}` or `ecosystems={"node"}` to `scan()`,
`analyze()`, or `remote_analysis()` to restrict discovery. The `"node"`
ecosystem covers both JavaScript and TypeScript. Selection also limits
workspace discovery and remote file requests; omitted ecosystems do not
contribute applications, facts, or diagnostics. `None` keeps the default of
analyzing both ecosystems.

Each `FileEntry` contains a path and its repository-reported size, or `None`
when the size is unknown. Kenbun does not request entries known to exceed
`max_file_bytes` and still validates the actual content supplied to `update()`.
Each `FileRequest` contains a path, reason, and priority. `max_files` bounds
requested file contents across every analysis round. The caller owns remaining
transport-specific metadata and returns `bytes` or `None` for every requested
path. `scan()` walks a real directory; `analyze()` remains the pure, stateless
primitive beneath `remote_analysis()`. Both analysis modes produce schema v3
`ScanResult` objects with deterministic ordering and canonical JSON.

## Supported detection

Python applications:

- FastAPI, including static entrypoint resolution and dependency diagnostics.
- Django and Flask identity detection. Kenbun does not infer their entrypoints
  yet.

Node applications:

- Next.js, Astro, Nuxt, SvelteKit, TanStack Start, SolidStart, and legacy
  Remix.
- Standalone Vite applications: the same directory must directly depend on
  Vite and contain `index.html`. A Vite config and build script are optional.
- React, Vue, Svelte, and Solid as supporting UI-framework facts.

React Router Framework Mode is not detected as an application yet.

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

- `ScanResult` contains protocol status/completeness, ordered `file_requests`,
  scan paths, optional `Workspace`, ordered `applications`, and diagnostics.
- `Application` contains `technologies`, optional entrypoint, one or more
  ecosystem-specific `DependencySet` values, explicit `build_scripts`, Python
  and Node runtime metadata, evidence, and local diagnostics.
- `DependencySet` and `BuildScript` report an inferred package-manager name
  and, when `package.json#packageManager` supplies one, its declared version.
- `Technology` has a normalized name, kind (`language`, `framework`,
  `ui-framework`, `integration`, or `build-tool`), role (`primary` or
  `supporting`), confidence, and evidence.
- `BuildScript` records the explicit `build` script as data: the raw command,
  optional safely parsed argv, optional package-manager facts, and source.

See the [v3 specification](docs/spec.md) for the normative model and detection
rules. See [Architecture](docs/architecture.md) for the internal detector and
assembly boundaries.

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
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
uv run ruff check .
uv run ruff format --check .
uv run pytest -q
cargo deny check
```
