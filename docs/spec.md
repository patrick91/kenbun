# kenbun — specification (v0.1, draft 2)

`kenbun` is a static-analysis library, written in Rust with Python bindings,
that identifies what kind of Python project lives inside a directory or a
partial set of files: which projects exist, whether they look like web apps
or libraries, which framework they use, where their entrypoint is, what they
depend on, and which environment variables they need — **without ever
importing or executing user code**.

Primary consumer: the FastAPI Cloud CLI and backend. kenbun itself stays
platform-agnostic: it reports facts and diagnostics; consumers decide what to
block, prompt, or auto-deploy.

> Draft 2 incorporates the adversarial review of draft 1: unified confidence
> rules, a defined `EnvVar` schema, factory/PEP 723 diagnostics, a terminating
> `want_files` protocol with `unavailable` + `tree_complete`, precise KB110 /
> KB302 semantics, upward-discovery path frames, and a determinism appendix.

## 1. Consumers and modes

1. **Deploy CLI (local directory).** `scan(path)` before upload: find deploy
   targets, validate user configuration, surface problems that would
   otherwise fail at build or runtime.
2. **Backend, deploy-time (full tree).** Same scan over an extracted GitHub
   tarball.
3. **Backend, bulk classification (partial files).** Right after a user
   connects GitHub, classify each repo as "uses FastAPI or not" from a tree
   listing plus a handful of fetched files. This mode drives the core
   architecture: the engine must work over a *virtual file set* and be able
   to tell the caller which files it wants next.

## 2. Hard constraints

- **Never import or execute user code.** All analysis is static (AST, TOML,
  file-tree shape).
- **Sans-IO core.** The detection engine operates on an abstract file
  provider. Direct filesystem walking is a frontend concern.
- **Never raise on malformed input.** Broken TOML, unparseable Python,
  contradictory configs degrade to structured diagnostics, never exceptions.
  Rust panics are caught at the FFI boundary and become KB900.
- **Deterministic and pure.** Same inputs → byte-identical output (§14).
  Safe to retry, safe to cache by content hash.
- **Prefer safe uncertainty over confident wrongness.** When a result cannot
  be established statically, report lower confidence plus a diagnostic — do
  not guess.

## 3. Public Python API

```python
import kenbun

# Mode 1/2: a real directory
result = kenbun.scan(
    root,                      # str | PathLike
    target_dir=None,           # user-configured app directory, relative to root (validated)
    entrypoint=None,           # user-configured "module:attr" (validated)
    max_files=None,            # scan budget; exceeded → KB802 + partial result
    follow_symlinks=False,
)

# Mode 3: a partial, virtual file set (GitHub API)
result = kenbun.analyze(
    files,                     # Iterable[FileEntry(path, size, blob_sha=None)] — known tree
    contents,                  # Mapping[str, bytes] — fetched, COMPLETE file contents
    absent=(),                 # paths positively known NOT to exist (negative evidence)
    unavailable=(),            # paths that exist (or were requested) but will never be
                               #   provided: fetch errors, oversized, LFS pointers,
                               #   truncated blobs. No evidence value; never re-requested.
    tree_complete=False,       # True iff `files` lists the ENTIRE repo
    target_dir=None,
    entrypoint=None,
)
```

Both return a `ScanResult`. `analyze` is stateless and pure: callers loop
`analyze → fetch result.want_files → analyze` until `status == "complete"`
or their budget runs out. Every round returns a well-formed result.

**Path canonicalization.** All paths (inputs and outputs) are `/`-separated,
relative, NFC-normalized, with no leading `./`. Inputs are normalized before
use. Input contradictions resolve as:

| Contradiction | Resolution |
|---|---|
| path in both `contents` and `absent` | treated as fetched; KB803 info |
| `contents` key not in `files` | a `FileEntry` is synthesized |
| `len(contents[p]) != FileEntry.size` | contents are trusted; size is advisory |
| duplicate `FileEntry` paths | first wins; KB803 info |
| truncated blob | must NOT be passed in `contents` — put the path in `unavailable` |

**`want_files` invariants (termination).** `want_files ⊆ files ∖ (contents ∪
absent ∪ unavailable)`; when `tree_complete=False`, kenbun may additionally
request speculative paths not in `files` (e.g. `pyproject.toml` when no tree
was given — this is how Tier-0 probing works with `files=[]`). A path never
reappears in `want_files` once it is in `contents`, `absent`, or
`unavailable`. Therefore, if the caller answers every request (with content,
`absent`, or `unavailable`), the want-set strictly shrinks each round and the
loop terminates — in practice within 3 rounds (manifests → members/entry
candidates → settings modules).

`target_dir`/`entrypoint` are the *hints input*: kenbun does not read any
platform config; the CLI reads its own config and passes values in. Hints
are validated, not trusted (§10). A hinted `target_dir` overrides default
exclusions (§6.1) — an explicit hint is always inspected, with an info
diagnostic when it points somewhere normally skipped.

`analyze` enforces the same resource budgets as `scan` (§14) — it is the
endpoint that ingests untrusted repo content. `FileEntry.blob_sha` is a
caller-side cache key: kenbun never interprets it, but echoes it on
`WantFile` entries so callers can fetch by SHA and cache analysis by content.
Git tree entries that are symlinks or submodules are treated as
`unavailable` (KB803 info).

All results are typed PyO3 classes with `.pyi` stubs (`py.typed` shipped),
plus `result.to_json()` producing versioned, canonical JSON (§14) the
backend can persist verbatim.

## 4. Output model

```text
ScanResult
├─ schema_version: int                    # bumped on breaking JSON changes
├─ root: str                              # the scan root as given
├─ upload_root: str                       # deployment context root, relative to `root`
│                                         #   ("." unless an ancestor workspace was found,
│                                         #    e.g. "../.." — see §7)
├─ scan_origin: str                       # `root` relative to upload_root (e.g. "apps/api")
├─ status: "complete" | "needs_files"
├─ want_files: [WantFile]                 # empty when status == "complete"
├─ input: {mode: "fs" | "virtual", files_seen: int, complete: bool}
├─ workspace: Workspace | None
├─ projects: [Project]
├─ deploy_targets: [DeployTarget]         # flattened, ranked (§6.5)
├─ classification: Classification         # cheap verdict for bulk mode (§11)
└─ diagnostics: [Diagnostic]              # union of all levels, deduped (§14)

# All paths below are relative to upload_root.

Workspace
├─ kind: "uv"
├─ path: str
├─ virtual_root: bool                     # [tool.uv.workspace] without [project]
└─ members: [str]                         # declaration/glob-expansion order (§14)

Project
├─ path: str
├─ name: str | None                       # [project.name] / [tool.poetry.name]
├─ roles: ["webapp" | "library" | "example" | "test-support"]   # not mutually exclusive
├─ frameworks: [str]
├─ deploy_targets: [DeployTarget]
├─ dependencies: Dependencies | None      # §8
├─ env_vars: [EnvVar]                     # §9
├─ python: {requires_python: str|None, version_pins: [{source, value}]}
├─ evidence: [Evidence]
└─ diagnostics: [Diagnostic]

DeployTarget
├─ framework: "fastapi"                   # v0.1: only value with a resolver (§12)
├─ form: "project" | "script"             # script = PEP 723 single file; consumers that
│                                         #   don't support single-file deploys filter on this
├─ project_path: str                      # == the builder's APP_DIR (relative to upload_root)
├─ entrypoint: Entrypoint | None          # None ⇒ framework detected, entry unresolved (KB103)
├─ confidence: "high" | "medium" | "low"
├─ recommended: bool                      # exactly one true when any target exists (§6.5)
├─ env_vars: [EnvVar]                     # aggregated (§9)
├─ evidence: [Evidence]
└─ diagnostics: [Diagnostic]

Entrypoint
├─ kind: "asgi" | "wsgi"                  # v0.1 emits only "asgi"
├─ module: "app.main"                     # for re-exports: the re-exporting package (§6.3)
├─ attribute: "app"
├─ is_factory: bool                       # true ⇒ KB112 attached, confidence ≤ medium
├─ import_root: str                       # dir the module path is relative to (handles src/)
├─ source: "hint" | "tool-fastapi" | "inferred"
└─ as_string: "app.main:app"              # the uvicorn / fastapi-run form, derived

EnvVar
├─ names: [str]                           # ≥1; >1 = alias group, ANY one satisfies it
├─ required: bool                         # per the rules in §9
├─ case_sensitive: bool
├─ value_kind: "scalar" | "json"          # json = nested model expected as one JSON var
├─ has_default: bool
├─ default_is_computed: bool              # default_factory or non-literal default
├─ source: "pydantic-settings" | "pydantic-settings-v1" | "os-environ"
│          | "os-getenv" | "starlette-config" | "django-environ" | "decouple"
├─ origin: {path, span, symbol}           # symbol e.g. "Settings.database_url"
└─ confidence: "high" | "medium" | "low"

Diagnostic
├─ code: "KB301"                          # stable registry, §5
├─ severity: "error" | "warning" | "info"
├─ message: str                           # human rendering; NOT for machine branching
├─ path: str | None
└─ span: {start_line, start_col, end_line, end_col} | None

Evidence
├─ kind: "dependency-declared" | "app-instance" | "factory-function"
│        | "config-entrypoint" | "marker-file" | "filename-convention"
│        | "lockfile" | "pep723-block" | "framework-import" | "router-usage"
│        | "re-export" | "mounted-app" | "runner-up-candidate"
├─ path: str
├─ span: {...} | None
└─ detail: str

WantFile
├─ path: str
├─ reason: "manifest" | "entry-candidate" | "settings-module" | "workspace-member" | "lockfile"
├─ priority: int                          # 0 = fetch first
├─ max_bytes: int                         # if the full file can't be provided within this,
│                                         #   mark it `unavailable` — do NOT truncate
└─ blob_sha: str | None                   # echoed from FileEntry when known

Classification
├─ python: "yes" | "no" | "unknown"
├─ uses_fastapi: "yes" | "likely" | "no" | "unknown"
└─ primary: {path, evidence} | None
```

There is intentionally **no `command` field**: a detection library cannot
know how a platform runs apps. Consumers derive commands from
`entrypoint.kind` + `as_string` + `is_factory`.

### 4.1 Confidence (normative — the single source of truth)

**high** requires ALL of:
1. framework dependency declared in a source the project's evident install
   path will install (§6.2 — KB301 conditions not met);
2. entrypoint resolved via a mechanism production discovery honors:
   a validated hint, `[tool.fastapi] entrypoint`, or a rule-3 conventional
   location (§6.3);
3. no cap applied.

**Caps (any one ⇒ at most medium):** KB301 (dependency not on the evident
install path), rule-4 entrypoint (resolvable statically but outside runtime
conventions — KB111), factory-only entrypoint (KB112), PEP 723 target
(KB114), KB110 ambiguity among top candidates, incomplete input for the
facts in question (in partial mode, high additionally requires that every
higher-priority candidate file is fetched or absent — §11).

**medium** = dependency declared but entrypoint unresolved, or any single
cap applied. **low** = convention-only signals (file names, framework
imports with no declared dependency — including a rule-3 file hit without
Layer-1 detection), or ≥2 caps.

| Level | Suggested consumer behavior |
|---|---|
| high | may act without confirmation |
| medium | confirm with the user |
| low | always confirm |

## 5. Diagnostics registry

Codes are stable across releases (ruff-style). Messages may be reworded
freely; consumers branch on codes only. Severity contract: `error` = this
will fail at build or runtime; `warning` = likely mistake, confirm;
`info` = FYI. Whether to block is always the consumer's decision. New codes
may be added in any release; existing codes are never renumbered or
repurposed. Every behavior in this spec that "notes" or "suggests" something
does so through a registered code.

| Code | Sev | Meaning |
|---|---|---|
| KB100 | error | no Python project found in scan root |
| KB101 | warning | multiple web frameworks detected in one project (identity specs, §12) |
| KB102 | error | Python project found, but nothing deployable |
| KB103 | error | framework dependency declared but no app object found (a placeholder target with `entrypoint=None` is still emitted so pickers can offer manual entry) |
| KB104 | info | APIRouter usage found but no FastAPI app (router-only module) |
| KB110 | warning | multiple deploy-target candidates not ordered by deterministic conventions (§6.3 definition) |
| KB111 | info | entrypoint resolvable statically but not discoverable by runtime conventions — suggest `[tool.fastapi] entrypoint` |
| KB112 | warning | factory-only entrypoint: convention-based runners (`fastapi run`) call instances, not factories |
| KB114 | warning | PEP 723 inline dependencies require a script-aware runner; standard project installers will not install them |
| KB115 | info | scan origin is inside a non-deployable workspace member; deploy targets exist in sibling members |
| KB200 | error | Python syntax error (path + span) |
| KB201 | error | pyproject.toml is not valid TOML (path + span) |
| KB202 | warning | pyproject.toml has no `[project]` table (and is not a workspace root) |
| KB203 | warning | unparseable requirements line (path + span) |
| KB204 | warning | multiple PEP 723 `script` blocks in one file; inline metadata ignored |
| KB300 | warning | both pyproject.toml and requirements.txt declare dependencies at the same root |
| KB301 | warning | framework dependency not on the evident install path (§6.2 conditions) |
| KB302 | error | uv.lock is provably out of sync with pyproject.toml (conservative check, §8; absence of KB302 is not proof of freshness) |
| KB303 | warning | dependency likely requires building from source (e.g. psycopg2 → psycopg2-binary as drop-in, or migrate to psycopg[binary]) |
| KB304 | warning | third-party import not found in declared dependencies (v1.1) |
| KB305 | warning | multiple lockfiles present (e.g. uv.lock + poetry.lock) |
| KB306 | info | non-uv package manager detected (poetry/pipenv/pdm) — migration hint |
| KB307 | error | no installable dependency source for this deploy target: dependencies declared only in tool-specific tables (e.g. `[tool.poetry.*]`) or inline metadata, with no lockfile, no `[project.dependencies]`, no pylock/requirements |
| KB308 | warning | project may not be installable as a package (no `[build-system]` or no discoverable package dir); editable-install fallbacks can fail at build |
| KB310 | warning | project-local import not found in the scanned tree (v1.1) |
| KB400 | error | workspace member glob matched a non-hidden, non-ignored directory without pyproject.toml |
| KB401 | error | nested workspace: member declares its own `[tool.uv.workspace]` |
| KB402 | warning | workspace members glob matched nothing |
| KB403 | warning | target_dir is inside a workspace but is not a member |
| KB500 | error | target_dir does not exist |
| KB501 | error | target_dir escapes the scan root (lexical check after normalization; fs mode also resolves symlinks) |
| KB502 | error | target_dir contains no Python project |
| KB503 | error | entrypoint module not found under any computed import root |
| KB504 | error | entrypoint attribute not found at module level |
| KB505 | warning | entrypoint attribute exists but does not look like an app object |
| KB600 | warning | env-var extraction unreliable for a settings class (dynamic prefix, custom sources, unknown base…) |
| KB601 | warning | `env_file` declared but file not present in scanned set |
| KB602 | warning | `.env` file present in the scanned tree (secret-leak adjacent) |
| KB603 | info | `secrets_dir` declared; contents unknowable statically |
| KB700 | warning | contradictory Python version pins (requires-python vs .python-version vs lockfile) |
| KB701 | info | no Python version constraint found anywhere |
| KB800 | info | analysis ran on incomplete input; see want_files |
| KB801 | info | workspace member resolution incomplete: tree listing missing or incomplete |
| KB802 | warning | scan budget exceeded (max_files); result is partial |
| KB803 | info | input treated as unavailable or normalized (symlink, submodule, oversized, truncated, contradictory entry) |
| KB900 | error | internal analyzer error (caught panic); result may be incomplete |

## 6. Detection semantics

### 6.1 Scan exclusions

Matched by basename at any depth, in both fs and virtual modes.

- **Unconditional:** `.git`, `.hg`, `.venv`, `venv`, `.tox`, `.nox`,
  `node_modules`, `__pycache__`, `site-packages`, `.eggs`, `*.egg-info`,
  `.mypy_cache`, `.ruff_cache`, `.pytest_cache`.
- **Conditional (real code can legitimately use these names):** `env`,
  `build`, `dist` are skipped only when they contain venv/build markers
  (`pyvenv.cfg`, `bin/activate`, `PKG-INFO`, `*.dist-info`, `*.whl`) or are
  gitignored; otherwise they are scanned normally.
- In fs mode, `.gitignore` is honored (the `ignore` crate). Symlinks are not
  followed by default.
- A hinted `target_dir` is **always** inspected, even inside an excluded
  directory (info diagnostic attached) — validation must never lie.
- Directories named `tests`, `test`, `docs`, `examples`, `example` are
  scanned; any project/target under them gets
  `roles += ["example"|"test-support"]`. **An example-role target is never
  `recommended` while any non-example target exists, regardless of
  confidence.**

### 6.2 Layer 1 — manifest detection (cheap; works on partial input)

A declarative, per-framework spec (data, not code):

```text
FrameworkSpec("fastapi"):
  dependency_names: {fastapi, fastapi-slim}          # PEP 503-normalized comparison
  weaker_signals:   {starlette}                      # evidence only, never "FastAPI"
```

Sources parsed, in all of which dependency names are compared
**PEP 503-normalized** (lowercase, `[-_.]+` → `-`; specifier name cut at the
first of `[`, whitespace, version operator, `;`, `@`; extras recorded as
evidence; never substring matching):

- `pyproject.toml`: `[project.dependencies]`,
  `[project.optional-dependencies].<extra>`, `[dependency-groups]` (PEP 735,
  `include-group` expanded with a cycle guard), `[tool.uv.dev-dependencies]`,
  `[tool.poetry.dependencies]`, `[tool.poetry.dev-dependencies]`,
  `[tool.poetry.group.<name>.dependencies]` (keys as names; the key `python`
  is a version constraint, not a package).
- `requirements*.txt` at a project root **and** `requirements/*.txt`:
  recursive `-r`/`-c` includes (paths relative to the containing file,
  depth ≤ 5), `-e` editables, direct refs and `#egg=` extraction. Files whose
  name matches `*dev*`, `*test*`, `*lint*`, `*doc*` are treated like
  non-default groups for KB301 purposes.
- Lockfiles: `uv.lock`, `pylock.toml`, `poetry.lock`, `Pipfile`
  (`[packages]`/`[dev-packages]` keys).
- **PEP 723** inline blocks (canonical regex; more than one block → KB204,
  block ignored) in candidate `.py` files. A file with a valid block whose
  `dependencies` include a framework is a **standalone deploy target** with
  `form = "script"` (entrypoint module = file stem, inline metadata
  overrides the enclosing project for that file) — always with **KB114**
  attached and confidence capped at medium, because standard project
  installers never read inline metadata. Platforms that do not support
  single-file deploys (FastAPI Cloud today) filter on `form == "script"`;
  keeping the detection lets the CLI say "this is a PEP 723 script, which
  this platform can't deploy" instead of "nothing found".

**KB301 (evident-install-path rule).** A framework dependency triggers KB301
— and caps the target at medium — only when it is *not* on the install path
the project itself makes evident:

- `uv.lock` present ⇒ default-synced groups count as installed (uv syncs the
  `dev` group by default; `tool.uv.default-groups` respected when literal).
  Optional-dependencies, non-default groups, and marker-guarded deps trigger
  KB301.
- No lockfile ⇒ only `[project.dependencies]` counts (PEP 621 installers
  read nothing else). Everything else — including poetry tables — triggers
  KB301, and **KB307 (error)** when *no* installable source declares the
  framework at all (e.g. a classic Poetry 1.x project with no
  `[project]` table and no requirements.txt: the platform installs nothing
  and the app cannot boot).

### 6.3 Layer 2 — entrypoint resolution (AST; ruff parser)

Ordered rule table. Rules are tried in order per project; later rules still
run to collect evidence and detect KB110.

| # | Rule | Entrypoint source | Confidence contribution |
|---|---|---|---|
| 1 | Explicit `entrypoint=` hint from the caller — validated (§10), never trusted | hint | high (or KB503/504/505) |
| 2 | `[tool.fastapi] entrypoint = "m:a"` in the project's pyproject.toml — validated; this is what `fastapi run` honors in production | tool-fastapi | high |
| 3 | fastapi-cli's exact search order: `main.py`, `app.py`, `api.py`, `app/main.py`, `app/app.py`, `app/api.py` (relative to project dir) | inferred | high **iff** Layer 1 detected the framework (else low: convention-only) |
| 4 | Extended candidates fastapi-cli does *not* try: for each package dir (`__init__.py` present) directly under the project root or `src/`, in byte order: `<pkg>/main.py`, `<pkg>/app.py`, `<pkg>/api.py`, `<pkg>/__init__.py` | inferred | medium + **KB111** |

Within a candidate module (all statically, from the AST):

- **Instance bindings:** module-level `x = FastAPI(...)` through any import
  form (`from fastapi import FastAPI`, aliased `as F`, `import fastapi` +
  `fastapi.FastAPI()`). Additionally, module-level `x = f(...)` where `f` is
  a factory (below) defined in the same module or imported one hop counts as
  an **instance binding** (`is_factory=false`) — this covers the dominant
  `app = create_app()` idiom.
- **Variable precedence** mirrors fastapi-cli: `app`, then `api`, then
  remaining candidates in alphabetical order (its `dir()`-based fallback).
- **Factories:** a function whose return annotation resolves to the imported
  FastAPI symbol, or whose every return statement returns a local bound to
  `FastAPI(...)`. A factory with no module-level instance binding anywhere in
  the project yields a target with `is_factory=true`, **KB112**, confidence
  ≤ medium — convention-based production runners call instances, not
  factories. A module-level instance always beats a factory.
- **One-hop re-exports:** supported forms are `from .X import name [as
  alias]` and the absolute equivalent when the first segment matches the
  package's own dotted path. The *exported* name participates in variable
  precedence; the reported `Entrypoint.module` is the **re-exporting
  package** (what an importer binds), with the defining module recorded as
  `re-export` evidence. `is_factory`/binding-kind propagate through the hop.
  Exactly one hop, counted in re-export edges; a hop landing on another
  re-export stops (evidence only). `from .x import *` and imports inside
  `try/except` are never followed (evidence only).
- **Mounted sub-apps:** `parent.mount(..., child)` — the root app is the
  target; mounts become `mounted-app` evidence.
- **Routers:** `APIRouter` usage alone never creates a target (KB104 when
  routers exist but no app does anywhere).
- Dotted module paths are computed by walking up `__init__.py` packages from
  the file (exactly like fastapi-cli); `import_root` is the first
  non-package ancestor. PEP 420 namespace packages (no `__init__.py`) are
  not supported for path computation in v0.1 (evidence note only).
- Any candidate file that fails to parse → KB200 with span (the #976 syntax
  gate, from the same pass, free).

**KB110 (ambiguity), defined.** Within one project, KB110 fires **only**
when two or more candidate modules would each independently produce a
target and they are *not* ordered by the deterministic search: rule-4-only
multi-hits across different packages, or a rule-3 hit plus a rule-4 hit in
different packages. Multiple rule-3 files (`main.py` and `app.py` both with
apps) and `app` vs `api` in one module are **not** ambiguous — production
behaves deterministically; losers are recorded as `runner-up-candidate`
evidence. Across projects, multiplicity is expressed by ranking +
`recommended`; KB110 additionally fires at top level only when the top two
non-example targets tie on confidence tier **and** scan-origin affinity
(§6.5) does not single one of them out.

### 6.4 Outcome codes for "nothing found"

The consumer must be able to distinguish, machine-readably: empty/no Python
(KB100) vs Python-but-nothing-deployable (KB102) vs
framework-declared-but-no-app-object (KB103 — placeholder target emitted
with `entrypoint=None` so pickers can offer manual entry; without
intervention the deploy fails, hence error) vs candidates-but-ambiguous
(KB110). Four states, four different CLI messages.

### 6.5 Ranking and `recommended`

`deploy_targets` sort order (total, deterministic):

1. **scan-origin affinity**: targets whose `project_path` is equal to or an
   ancestor of `scan_origin` — the project the user is standing in.
   Explicit location beats every heuristic (including the example rule:
   running inside `examples/demo` means you want the demo);
2. non-example over example/test-support (§6.1);
3. `form = "project"` over `form = "script"`;
4. confidence desc;
5. shallower `project_path` (fewer segments);
6. workspace member order (declaration order of the `members` array; glob
   expansions in byte order);
7. `project_path` bytes ascending.

Exactly one target has `recommended = true` whenever any target exists.
Whether the consumer auto-deploys it without prompting is consumer policy.

The target **set** is origin-independent: scanning anywhere inside the same
workspace produces the same projects and targets (upward discovery finds the
same root). Only ordering, `recommended`, `scan_origin`, and
origin-dependent diagnostics change. When the scan origin sits inside a
non-deployable member (e.g. a library) while targets exist in sibling
members, KB115 is emitted so the CLI can redirect the user.

## 7. uv workspaces

kenbun reproduces uv's discovery semantics statically:

- A workspace root is a `pyproject.toml` with `[tool.uv.workspace]`.
  `members`/`exclude` are root-relative globs (Rust `glob` semantics);
  exclude wins; hidden entries skipped; **the root is always a member**. A
  root without `[project]` is a *virtual root* (`virtual_root = true`,
  itself non-deployable).
- **Upward discovery (fs mode only):** when the scan root sits inside a
  member, kenbun walks real ancestors like uv does: nearest pyproject wins
  if it is a root; otherwise an ancestor root whose members include (and
  exclude does not) the project dir; else the project is its own implicit
  single-member workspace. **In virtual mode the provided root is the
  outermost boundary** — no ancestor walking, no `../` requests; when the
  root itself looks like a member (`{workspace = true}` sources with no
  reachable root), KB801 (info) is emitted and analysis proceeds treating
  the root as standalone.
- **Path frames.** When upward discovery finds an ancestor root, the result
  sets `upload_root` (e.g. `"../.."`) and `scan_origin` (e.g. `"apps/api"`),
  and *all* result paths are relative to `upload_root`. Example — CLI run
  from `repo/apps/api`:

  ```text
  root:         /home/u/repo/apps/api     (as given)
  upload_root:  "../.."                   → upload from /home/u/repo
  scan_origin:  "apps/api"
  workspace:    {path: ".", members: [".", "apps/api", "packages/core"]}
  target:       {project_path: "apps/api"}   → builder APP_DIR = "apps/api"
  ```

- Member globs matching a non-hidden, non-ignored dir without
  `pyproject.toml` → KB400; a member with its own `[tool.uv.workspace]` →
  KB401 (uv errors on both; kenbun diagnoses and continues).
- `[tool.uv.sources]`: `{workspace = true}` confirms membership;
  `{path = ...}` links projects in non-workspace monorepos (recorded as
  project relationships; used by env-var aggregation, §9).
- Consumers upload the whole tree from `upload_root`; kenbun does not
  compute minimal upload subsets.

## 8. Dependency metadata (issue #2340)

Per project, fully serializable:

```text
Dependencies
├─ package_manager: "uv" | "poetry" | "pdm" | "pipenv" | "pip" | "unknown"   (+ evidence)
├─ manifests: [{path, kind}]
├─ lockfiles: [{path, kind, parsed}]
├─ declared:  [{name, raw, specifier, extras, markers,
│               group: "project" | "optional:<extra>" | "group:<name>" | "dev",
│               source: {path, span}}]
└─ resolved:  [{name, version, source, marker: str | None}]
```

- `resolved` is parsed from `uv.lock` and `pylock.toml` in v0.1
  (`poetry.lock` is identity-evidence only until v1.1 — it is a different
  format and not on the reference platform's install path). The same name
  may appear at multiple versions (marker forks); consumers must not assume
  uniqueness — `marker` disambiguates when present.
- **KB302 (lock drift), exact check:** compare PEP 503-normalized names from
  `[project.dependencies]` (and each workspace member's) against the
  member's recorded dependency metadata in `uv.lock`. A name present in
  pyproject but absent from the lock metadata (or vice versa) ⇒ KB302
  error — this conservative case is exactly what makes `uv sync --locked`
  fail. Specifier-only mismatches ⇒ warning-level message under the same
  code. Unknown lock `version`/`revision` ⇒ skip the check + info. Absence
  of KB302 is never proof of freshness.
- Python version facts feed `python.*` and KB700/KB701; contradictions are
  diagnosed, not resolved.

"Final installed versions" remain a build-time concern owned by the
platform; kenbun provides declared + lockfile-resolved.

## 9. Environment variables (issue #2886)

Static, refuse-to-guess extraction; every var carries a concrete `origin`.
The `EnvVar` schema is defined in §4.

**Layer 1 — pydantic-settings.** A project-local symbol graph (bounded:
files within the project and its linked projects only, per-file parse cap,
inheritance chains ≤ 3 hops) resolves classes transitively subclassing
`pydantic_settings.BaseSettings` (v2) or `pydantic.BaseSettings` (v1 —
selected by the declared pydantic major version). Only literal configuration
is evaluated: `env_prefix`, `case_sensitive`, `env_nested_delimiter`,
`env_file`, `secrets_dir` from `SettingsConfigDict`/`class Config`;
`Field(default, default_factory, alias, validation_alias=
AliasChoices(...literals))`; for v1 additionally `Field(env="X")` and
`class Config.fields = {"f": {"env": ...}}` (explicit `env=` names are used
verbatim — v1 does not apply `env_prefix` to them).

- **Required** iff no default and no default_factory. Under v2,
  `Optional[str]` with no default **is required**; under v1 it defaults to
  None (not required).
- Names: `env_prefix + field_name`, reported UPPERCASE when
  case-insensitive, literal when `case_sensitive=True`. Aliases replace the
  derived name (prefix does not apply to aliases); `AliasChoices` fills
  `names` as an any-one-satisfies group.
- Nested models expand only when project-defined, delimiter literal, depth
  ≤ 2; otherwise the parent var is one `value_kind="json"` entry.
- Anything dynamic — computed prefix, `settings_customise_sources`,
  third-party base class, `create_model`, validators — ⇒ KB600 for the
  class; no fabricated vars. `default_factory` ⇒ `has_default=true,
  default_is_computed=true`.

**Layer 2 — accessor signals** (single AST visitor): `os.environ["X"]` →
required; `os.getenv("X")` / `.get("X")` → referenced (required=false — no
flow analysis); starlette `Config()("KEY")`, django-environ,
python-decouple without default → required; `load_dotenv()` and committed
`.env.example` → hints (evidence, not EnvVars).

**Aggregation.** `DeployTarget.env_vars` = the union of its project's vars
and those of every project reachable via `{path = ...}` **or**
`{workspace = true}` dependency links (a workspace lib's Settings class is
typically consumed by the app). Dedup key: the sorted `names` tuple;
`required` = any(required); order: first name, bytes ascending.

Consumers diff `required` vars against the platform's configured vars and
report "missing required env var `APP_DB_URL` (app/settings.py:14,
Settings.db_url)" before upload. kenbun never claims a var is satisfied at
runtime (env_file/secrets_dir/kwargs can satisfy them — KB601/KB603).

## 10. Validation mode (issues #976, #2366)

When `target_dir`/`entrypoint` hints are passed, kenbun validates instead of
trusting: KB500–KB505. `target_dir` overrides exclusions (§6.1); KB501 is a
lexical check after normalization (fs mode also resolves symlinks).
Malformed pyproject.toml anywhere degrades to KB201/KB202 attached to that
project.

Static loadability, v0.1 scope: syntax check (KB200) on every candidate
entry module and every file in the entrypoint's project-local import closure
(bounded); entrypoint resolution per §6.3; KB308 when the project's evident
install path requires installing the project itself but no build backend /
package layout supports it. v1.1: the import closure additionally yields
KB304 (undeclared third-party import — needs the curated import-name →
distribution mapping, e.g. `PIL` → `pillow`) and KB310 (project-local import
that resolves to no file in the tree).

## 11. Partial-input mode and the `want_files` protocol

- `files` = the known tree (e.g. GitHub's recursive tree API);
  `tree_complete` says whether it is exhaustive (the tree API's
  `truncated: true` ⇒ pass `tree_complete=False`). **Absence is evidence**
  (an entry in `absent`, or a path missing from a complete tree);
  **unfetched/unavailable is uncertainty** — never converted into a negative
  claim.
- Tier-0 probing needs no tree: `files=[]`, `tree_complete=False`, contents
  = the fetched root manifests, `absent` = the 404s. kenbun's speculative
  requests then drive deeper rounds if the caller continues.
- Request priority: manifests → lockfiles → workspace-member manifests →
  entry-candidate modules → settings modules.
- **Two testable guarantees** (replacing draft 1's blanket monotonicity):
  1. **Classification finality:** `uses_fastapi`/`python` values of `"yes"`
     and `"no"` never change as more input is supplied. `yes` only ever
     derives from fetched content (stable); `no` requires
     `tree_complete=True` *and* every manifest-tier want resolved (fetched,
     absent, or unavailable — unavailable manifests force `unknown`, not
     `no`).
  2. **High-confidence subset-soundness:** a `high`-confidence target
     reported on partial input persists (same `project_path`, same
     `entrypoint.as_string`) on any superset of that input. Enforced by the
     partial-input cap in §4.1: high is only granted when all
     higher-priority candidate files for that project are fetched or absent.
  Per-target confidence below high, `recommended`, and diagnostics carry
  **no** cross-round guarantee.

The recommended backend pipeline (GraphQL batch probe of ~100 repos/query →
recursive tree for the unresolved → blob fetch loop driven by `want_files`,
≤3 rounds, ~1–2 MB/repo cap → tarball only at deploy time) lives in the
platform; kenbun's contract is `analyze()` as specified.

## 12. Framework extensibility

v0.1 ships **entrypoint resolution for FastAPI only**, plus
**identity-only specs** for Django and Flask (dependency names + marker
files such as `manage.py` containing `DJANGO_SETTINGS_MODULE`) so that
`Project.frameworks`, `Classification`, and KB101 can name what they see
without resolving it. Adding full Django support later means an
`asgi.py`/`wsgi.py` resolver deriving `config.asgi:application` — never
`manage.py runserver`; dev servers are not deploy entrypoints.

## 13. Scope

### 13.1 Version roadmap (consolidated)

| | Contents |
|---|---|
| **v0.1** | scan/analyze APIs; FastAPI resolver + Django/Flask identity specs; uv workspace discovery; dependency metadata (declared + uv.lock/pylock resolved); env-var extraction (pydantic-settings v1/v2 + accessors); validation mode; syntax gate; diagnostics KB100–KB803, KB900 (minus v1.1 codes); partial-input mode with both guarantees |
| **v1.1** | import-closure checks (KB304, KB310); poetry.lock resolved parsing; Settings-instantiation call-site confidence boost; import-name→distribution mapping |
| **later** | Django/Flask resolvers; WSGI targets; additional workspace kinds |
| **never** | importing/executing user code; dependency resolution; run-command synthesis; platform config parsing; flow analysis for env vars; `setup.py` execution (string-scan evidence only); `AliasPath` |

## 14. Implementation notes

- **Crates:** `ruff_python_parser`/`ruff_python_ast`/`ruff_text_size`/
  `ruff_source_file` exact-pinned (`=0.0.x`, upgraded together, wrapped
  behind an internal module); `toml` + `serde` (deserialize-error spans feed
  KB201); `pep440_rs`/`pep508_rs`; `ignore` for the fs frontend only.
  rustpython-parser (dormant) and tree-sitter (weaker AST, C toolchain
  burden) were evaluated and rejected.
- **Architecture:** core engine consumes a `FileSet` trait (list + read);
  frontends: filesystem (ignore-walker) and virtual (maps from `analyze()`).
  One code path for all modes.
- **FFI:** scans run with the GIL released; panics are caught at the
  boundary → KB900.
- **Budgets (both entry points):** per-file parse cap (default 2 MB;
  oversized → unavailable + KB803), file-count budget (`max_files` →
  KB802), bounded `-r` recursion, no symlink following by default,
  path-escape rejection. These are DoS guards: kenbun ingests untrusted
  repo content.
- **Determinism appendix:**
  - Every list has a total order. Universal tie-break: path bytes
    ascending. `projects` by path; `diagnostics` by (path, span.start,
    code); `evidence` by (path, span.start, kind); `env_vars` by first
    name; `declared` by (name, source.path, span.start); `resolved` by
    (name, version); `want_files` by (priority, path); `workspace.members`
    by declaration order with glob expansions in byte order.
  - Top-level `diagnostics` = the union of scan-level and all
    project/target-level diagnostics, deduped by exact (code, path, span),
    in the order above.
  - fs walks are serial and byte-ordered so `max_files` truncation is
    reproducible across platforms.
  - `to_json()`: UTF-8, struct-declaration key order, compact separators —
    byte-identical for identical inputs at a given schema_version.

## 15. Testing strategy

- **Golden corpus** of fixture repos (single app, src layout,
  `app = create_app()`, factory-only, package `__init__` app, router-only,
  PEP 723 script, uv workspace, virtual root, non-workspace monorepo,
  poetry-classic (KB307), django+fastapi mixed, broken syntax, empty dir,
  requirements-only, both-manifests) with snapshot-tested JSON (insta).
- **Differential tests:** fastapi-cli's discovery invoked as a library
  (`get_import_data`) in a subprocess against importable fixtures, with the
  fastapi-cli version pinned; kenbun's inferred entrypoint must match.
- **Partial-input properties:** for every fixture, (a) replay the exact
  `want_files`-driven fetch sequence and assert both §11 guarantees at each
  round; (b) randomized subset sampling asserting high-confidence
  subset-soundness and classification finality.
- Python-side tests cover the FFI surface, typing (mypy over stubs), and
  canonical-JSON stability.

## 16. Issue mapping (fastapilabs/cloud)

| Issue | kenbun capability |
|---|---|
| #976 detect app exists/valid before upload | KB100/102/103 outcome codes; KB200 syntax gate; entrypoint static resolution; `target_dir` validation (KB500–502); KB308 |
| #960 dependency issues | KB300 (builder-true message: pyproject wins, requirements ignored), KB301, KB302, KB305, **KB307** |
| #2366 validate pyproject + directory | §10 validation mode; KB201/202; KB500–505 |
| #2886 pydantic settings | §9 env-var extraction |
| #2340 dependency metadata | §8 declared + lockfile-resolved, serializable |
| #2365 build/install UX (tracking) | #2052 → KB303 source-build warnings + KB700 version conflicts; #2080/#2367 → package_manager detection (KB306) feeding migrate-to-uv; #2366 → above |

## 17. Appendix — FastAPI Cloud builder alignment

Verbatim behavior of `backend/data/builder-context` (2026-07). kenbun stays
agnostic; the CLI owns platform-specific phrasing.

**install_dependencies.sh** — `if [ -f uv.lock ] || [ -f "$APP_DIR/uv.lock" ]`
(root **or** app dir):
`uv sync --frozen --no-install-project --no-install-workspace --directory
$APP_DIR`. Note: uv's default sync includes the `dev` group → KB301 must not
fire for default-group deps on this path. Otherwise, `cd $APP_DIR` and, in
order: `pyproject.toml` → `uv pip install -r pyproject.toml` (reads **only**
`[project.dependencies]` → KB301/KB307 rationale); `pylock.toml` →
`uv pip install -r pylock.toml`; `requirements.txt` →
`uv pip install -r requirements.txt`; else **bare
`uv pip install 'fastapi[standard]'`** — why empty dirs "deploy
successfully" (KB100/KB102 catch this pre-upload).

**install_project.sh** — with a lock: `uv sync --locked --directory
$APP_DIR` — a stale lock **fails the build** → KB302. Without a lock but
with `$APP_DIR/pyproject.toml`: `uv pip install --no-deps --editable
$APP_DIR` — an uninstallable package layout fails here → KB308. (This
editable install is also what makes unlocked src-layouts importable at
runtime.)

**Runtime** — `fastapi run --proxy-headers` with `WORKDIR /app/$APP_DIR`:
production discovery *is* fastapi-cli's, honoring `[tool.fastapi]
entrypoint`; §6.3 rules 1–3 predict it exactly. Rule-4 hits (KB111) and
factory-only targets (KB112) are precisely the cases `fastapi run` alone
cannot serve — the CLI should offer to write `[tool.fastapi] entrypoint`
or adjust the start command. Installer precedence **without uv.lock**:
pyproject.toml > pylock.toml > requirements.txt > bare fallback (note:
opposite of Nixpacks/Railway, who prefer requirements.txt). `APP_DIR` ==
`DeployTarget.project_path` relative to `upload_root`; the whole tree is
uploaded. The build image compiles C extensions (gcc, python3-dev,
libpq-dev in the Dockerfile) but source builds are slow → KB303 stays a
warning, not an error.
