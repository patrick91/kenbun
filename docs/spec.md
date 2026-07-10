# Kenbun specification (schema v1)

This document defines the public schema and detection behavior for Kenbun v1.
Kenbun is a static repository analyzer implemented in Rust with typed Python
bindings. It discovers applications and reports evidence; it does not make
deployment decisions.

The words **must**, **must not**, **should**, and **may** are normative.

## 1. Constraints and ownership

- Analysis must be static. Kenbun must not import repository modules, install
  dependencies, run package scripts, or execute configuration files.
- An unchanged filesystem must produce the same ordered objects and
  byte-identical `to_json()` output.
- Repository paths in the result use `/` separators and are relative to the
  effective scan/upload root unless a field says otherwise.
- Bad or contradictory repository content should produce typed diagnostics
  and partial facts rather than guesses.
- Kenbun reports application boundaries, technology evidence, dependency
  metadata, explicit build-script facts, and entrypoints where supported.
- Consumers own selection, prompting, command construction, and the decision
  that an application is supported or deployable. There is no `recommended`,
  `selected`, or generated runtime-command field.

## 2. Filesystem API

```python
def scan(
    root: str | os.PathLike[str],
    *,
    application_dir: str | None = None,
    entrypoint: str | None = None,
    max_files: int | None = None,
    follow_symlinks: bool = False,
    extra_ignore_files: list[str] | None = None,
) -> ScanResult: ...
```

`root` must identify a real directory. Schema v1 has no virtual-file API.

`application_dir` is an optional repository-relative hint supplied by a
consumer. Kenbun validates that it remains under the effective root, exists,
and matches a detected application. It does not suppress other
detected applications or mark the hinted application as selected. When an
`entrypoint` hint is also provided, it applies to the hinted application; in
the absence of `application_dir`, it applies to the application containing the
scan origin.

`entrypoint` uses `module:attribute` syntax and is currently interpreted only
by the FastAPI resolver. It is validated against statically parsed source.

`max_files` bounds the filesystem walk. Exceeding it returns the partial facts
with `KB802`. Symlinks are not followed unless `follow_symlinks=True`.
`extra_ignore_files` lets a caller apply deployment-specific ignore files in
addition to the built-in exclusions and `.gitignore`.

If the supplied root is inside a recognized workspace, Kenbun may discover the
workspace root upward. `root` remains the caller's original path;
`upload_root` and `scan_origin` describe that path relationship.

## 3. Public output model

All public records are frozen PyO3 classes and have corresponding type stubs.
`ScanResult.to_json()` emits compact schema-versioned JSON in declaration
order.

```text
ScanResult
├─ schema_version: int                    # exactly 1
├─ root: str                              # path supplied to scan()
├─ upload_root: str                       # "." or path from root to workspace root
├─ scan_origin: str                       # root relative to upload root
├─ workspace: Workspace | None
├─ applications: list[Application]        # sorted by application_dir
└─ diagnostics: list[Diagnostic]          # aggregate, deduplicated, stable order

Application
├─ application_dir: str                   # "." or upload-root-relative directory
├─ name: str | None
├─ technologies: list[Technology]
├─ entrypoint: Entrypoint | None
├─ dependencies: list[DependencySet]
├─ build_scripts: list[BuildScript]
├─ env_vars: list[EnvVar]
├─ python: PythonInfo | None
├─ evidence: list[Evidence]
└─ diagnostics: list[Diagnostic]

Technology
├─ name: str                              # normalized stable identifier
├─ kind: "language" | "framework" | "ui-framework"
│       | "integration" | "build-tool"
├─ role: "primary" | "supporting"
├─ confidence: "high" | "medium" | "low"
└─ evidence: list[Evidence]

DependencySet
├─ ecosystem: "python" | "node"
├─ package_manager: str | None
├─ manifests: list[ManifestRef]
├─ lockfiles: list[LockfileRef]
├─ declared: list[DeclaredDep]
└─ resolved: list[ResolvedDep]

BuildScript
├─ name: str                              # v1 emits only "build"
├─ command: str                           # exact package.json script value
├─ package_manager: str | None
├─ argv: list[str] | None                 # only when safely representable
└─ source: SourceRef

Workspace
├─ kind: "uv" | "npm" | "pnpm" | "yarn" | "bun" | "node" | "mixed"
├─ path: str
├─ virtual_root: bool
└─ members: list[str]
```

An `Application` may have both Python and Node dependency sets. `python` is
present only when Python metadata applies. `entrypoint` is optional because
only FastAPI has detailed entrypoint resolution in v1 and because a framework
can be detected even when its entrypoint is unresolved.

`Evidence` records a kind, path, optional span, and human-readable detail.
`Diagnostic` records a stable code, severity, message, optional path, and
optional span. Consumers must branch on codes, not messages.

## 4. Application boundaries

Kenbun recursively discovers candidate directories from supported manifests
and workspace membership. Generated and vendored directories such as `.git`,
virtual environments, caches, build output with generated markers, and
`node_modules` are excluded. `.gitignore` and caller-provided ignore files are
honored.

At most one `Application` is emitted for one `application_dir`:

1. Primary framework evidence at the directory qualifies the application.
2. Supporting languages, UI frameworks, integrations, build tools,
   dependency sets, and build scripts at that same directory attach to it.
3. Multiple primary frameworks at the same directory remain technologies on
   that one application and produce an ambiguity diagnostic.
4. A manifest in a nested directory is not attached to its parent. The nested
   directory must independently satisfy an application rule to be emitted.
5. A package that contains only library/tooling evidence is not an
   application.

Consequently, a same-root FastAPI and Vite frontend is one FastAPI application
with supporting Node/Vite facts, while independently qualified `backend/` and
`frontend/` directories are two applications.

## 5. Python detection

Python dependency evidence is read from supported `pyproject.toml` tables,
requirements files, Pipfile, and recognized lockfiles. Dependency names are
PEP 503 normalized. Poetry and PDM metadata is parsed as data; `setup.py` may
be string-scanned for weak evidence but is never executed.

The normalized Python framework technologies are:

| Dependency or marker | Technology | Behavior |
|---|---|---|
| `fastapi` or `fastapi-slim` | `fastapi` | Detailed static resolver |
| `django`, or a valid Django `manage.py` marker | `django` | Identity only |
| `flask` | `flask` | Identity only |

All are `kind="framework"` and `role="primary"`; Python is a supporting
language technology.

### 5.1 FastAPI entrypoint resolution

Resolution precedence is:

1. A validated `entrypoint` argument for the relevant application.
2. `[tool.fastapi].entrypoint` in its `pyproject.toml`.
3. FastAPI CLI's conventional file order: `main.py`, `app.py`, `api.py`,
   `app/main.py`, `app/app.py`, then `app/api.py`.
4. A deterministic static search outside those conventions, reported at lower
   confidence with `KB111`.

Python is parsed with an AST. The resolver recognizes `FastAPI(...)`
instances, common aliases, `app = create_app()`, factory-only functions, and a
one-hop package re-export. Attribute precedence is `app`, then `api`, then
lexical order. It never imports the module.

FastAPI technology confidence incorporates both dependency/install-path
evidence and entrypoint quality. Factory-only or non-conventional entrypoints
cap confidence and emit diagnostics. A declared FastAPI dependency with no
resolved object still produces the application with `entrypoint=None` and
`KB103`.

Django and Flask do not receive inferred `Entrypoint` values in v1.

## 6. Node detection

Node evidence is read from `package.json`, workspace manifests, and supported
lockfiles. Only declarative dependency and script data is used.

| Direct package signal | Normalized technology | Usual supporting facts |
|---|---|---|
| `next` | `nextjs` | React, JavaScript/TypeScript |
| `astro` | `astro` | JavaScript/TypeScript, optional UI framework |
| `nuxt` | `nuxt` | Vue, JavaScript/TypeScript |
| `@sveltejs/kit` | `sveltekit` | Svelte, Vite, JavaScript/TypeScript |
| TanStack Start package | `tanstack-start` | React or Solid, Vite, TypeScript |
| `@react-router/dev` plus config/build evidence | `react-router` | React and Vite |
| `@solidjs/start` | `solidstart` | Solid, JavaScript/TypeScript |
| `@remix-run/dev` | `remix` | React, JavaScript/TypeScript |

These normalized technologies are primary frameworks. `react`, `vue`,
`svelte`, and normalized `solid` are `ui-framework` technologies and normally
supporting.
`javascript` and `typescript` are language technologies. React Router used
only as a routing library must not qualify an application; legacy Remix
remains distinct from React Router Framework Mode.

React Router Framework Mode specifically requires a direct `@react-router/dev`
dependency plus at least one of: `react-router.config.*`, a Vite configuration
containing `@react-router/dev/vite`, or a `build` script directly invoking
`react-router build`.

### 6.1 Vite boundary

Vite is normally a `build-tool` technology. It qualifies a standalone
application with `role="primary"` only when all of the following exist at the
same directory:

1. Vite is a direct dependency.
2. An explicit `scripts.build` directly invokes `vite build`.
3. A root `index.html` exists.

Without all three, Vite may attach as supporting build tooling to an already
qualified application but must not create an application. This prevents a
backend asset pipeline or a Vite-built library from being reported as a
second deployable frontend.

### 6.2 Cross Inertia

The normalized `cross-inertia` technology has `kind="integration"` and a supporting
role. It is emitted for Cross Inertia only when the Python `cross-inertia`
dependency and a direct `@inertiajs/react`, `@inertiajs/vue3`,
`@inertiajs/svelte`, or `@inertiajs/vite` dependency occur at the same
application directory. Direct React, Vue, or Svelte dependencies may
additionally produce UI-framework evidence. Inertia never creates an
application by itself.

## 7. Workspaces and package managers

Kenbun recognizes:

- uv workspaces from `[tool.uv.workspace]`.
- npm, Yarn, and Bun workspaces from `package.json` workspace declarations,
  disambiguated by explicit manager or lockfile evidence.
- pnpm workspaces from `pnpm-workspace.yaml`.

Members are expanded deterministically and recorded in `Workspace.members`.
A workspace root without its own detected application has `virtual_root=True`.
Scanning from a member or a directory inside one may discover the containing
workspace upward and report all independently qualified member applications.

`Workspace.kind` names an unambiguous manager when one is known. It is `node`
when a Node workspace is valid but its manager is ambiguous or unknown, and
`mixed` when the same root declares both uv and Node workspaces.
Manager-specific facts remain optional on `DependencySet` and `BuildScript`.

Node package-manager inference uses this precedence:

1. The nearest `package.json` with an explicit `packageManager`, walking from
   the application directory toward the effective root.
2. Otherwise, the nearest directory with lockfile or workspace evidence, but
   only when exactly one of npm, pnpm, Yarn, or Bun is represented.

Conflicting evidence produces no inferred manager and a diagnostic. Kenbun
must not default to npm merely because `package.json` exists.

## 8. Build scripts

For a Node dependency set, v1 records only an explicitly declared
`scripts.build`. `BuildScript.command` preserves the raw string. `argv` is set
only for a simple command that can be represented safely without interpreting
shell control syntax; commands such as `tsc && vite build` keep their raw form
and use `argv=None`. `package_manager` follows the inference rules above and
may be `None`.

The script is a repository fact, not a recommendation. Kenbun never executes
it and does not choose whether a consumer should run it.

## 9. Diagnostics and determinism

Diagnostic codes are stable machine identifiers; severity and message are
presentation facts. Application diagnostics are also aggregated onto
`ScanResult.diagnostics`, deduplicated, and sorted. Existing code families
cover discovery (`KB100`–`KB112`), parsing (`KB200`–`KB203`), dependency consistency
(`KB300`), workspaces (`KB400`), hints (`KB500`), version conflicts (`KB700`),
and scan limits (`KB802`).

Applications sort by `application_dir`. Technologies, dependency metadata,
workspace members, evidence, and diagnostics use stable semantic or bytewise
orders. Filesystem enumeration order must not affect output. Serialization is
UTF-8 compact JSON with the public model's declaration order.

## 10. External acceptance fixtures

Unit and filesystem tests remain network-independent. The manual external
runner reads `tests/external_fixtures.json`, whose entries contain a fixture
name, GitHub `owner/repository`, full 40-character commit SHA, optional scan
subdirectory/arguments, and expected normalized projection.

```bash
uv run python scripts/check_external_fixtures.py
uv run python scripts/check_external_fixtures.py --fixture NAME
uv run python scripts/check_external_fixtures.py --offline
```

The runner downloads commit-addressed archives, rejects unsafe archive paths
and links, caches under `target/github-fixtures`, and scans without installing
or executing fixture code. It must never follow a mutable default branch.

## 11. Deferred capabilities

The following are intentionally outside schema v1:

- PHP and Laravel application detection.
- A virtual `analyze()` API or incremental `want_files` protocol.
- Runtime/build command selection, application recommendation, deployability,
  or platform support policy.
- Generic Inertia detection for stacks other than the same-root Cross Inertia
  integration.
- Detailed Django or Flask entrypoint resolution.
