0.9.3 - 2026-08-06
------------------

This release makes Python and Node framework detection easier to extend without
changing analysis behavior.

- Give Django, FastAPI, and Flask dedicated detector modules with explicit,
  statically dispatched calls from the framework orchestrator.
- Give Astro, Next.js, Nuxt, Remix, SolidStart, SvelteKit, and TanStack Start
  the same explicit per-framework structure on the Node side.
- Share sorted dependency lookup between detectors and keep FastAPI entrypoint
  resolution within the FastAPI module.

0.9.2 - 2026-08-06
------------------

This release adds CodSpeed performance tracking for repository analysis.

- Benchmark generated scale scenarios and representative fixture repositories,
  including a mixed Python and Node workspace with multiple frameworks.
- Run the benchmark workflow on Python 3.14 using deterministic CPU simulation.

0.9.1 - 2026-08-04
------------------

This release adds wheels and test coverage for Python 3.15, including
free-threaded builds, and updates PyO3 and Maturin for compatibility with the
new interpreter.

0.9.0 - 2026-08-03
------------------

Remote repository analysis can now identify symbolic links without fetching
their link-target blobs as ordinary files.

- Add the optional `is_symlink` field to `FileEntry`. Marked entries are
  validated but excluded before `FileSet` construction, so a linked manifest,
  ignore file, lockfile, or source file cannot invent repository facts.
- Never request or follow remote symlinks. Pre-supplied link-target contents
  are ignored, symlinks do not consume file budgets, and deliberately excluding
  them does not reduce completeness. This matches local scans with
  `follow_symlinks=False`.

0.8.0 - 2026-07-31
------------------

Exhausting a remote analysis's round budget no longer throws away what it found.

- `max_rounds` now ends the session instead of raising. `file_requests` becomes
  empty, the new `round_limit_reached` property becomes `True`, and `result`
  reports `completeness="partial"`. This brings it in line with `max_files` and
  `max_file_bytes`, which have always narrowed a result rather than failing it.
- Callers previously had to catch a bare `RuntimeError` and re-run `analyze()`
  to recover the facts the session already held. `result` is now reachable
  directly, so a bounded analysis costs nothing extra and cannot disagree with
  the session it came from.
- A `RuntimeError` from `update()` consequently always signals a broken
  invariant rather than an exhausted budget, so callers should no longer catch
  it. The result's `status` still reports `needs_files` when the analysis wanted
  more content; the session's decision to stop is reported separately by
  `round_limit_reached`.

0.7.0 - 2026-07-31
------------------

Kenbun can now bound how deep into a repository an analysis looks.

- Add the optional `max_depth` argument to `scan()`, `analyze()`, and
  `remote_analysis()`. Depth counts the directories above a file, so `main.py`
  is depth 0 and `app/main.py` is depth 1. Deployable applications sit near the
  top of a repository, so capping depth keeps vendored and generated trees from
  spending the file budget on paths that cannot contain an application. Local
  scans skip directories that can only contain deeper files.
- Treat depth as an exclusion in the same family as `node_modules` and `.venv`
  rather than as truncation, so it does not affect `completeness`. The limit
  describes where applications can live; calling it partial would mark ordinary
  repositories incomplete over paths that never mattered, and drown out the
  results that genuinely are partial.

0.6.0 - 2026-07-29
------------------

Kenbun can now focus repository analysis on Python or Node ecosystems
independently.

- Add the optional `ecosystems` argument to `scan()`, `analyze()`, and
  `remote_analysis()`, with typed support for selecting Python, Node, or both.
  Disabled detectors no longer contribute workspace framing, file requests,
  facts, or diagnostics.
- Group Python and JavaScript/TypeScript detection under explicit ecosystem
  modules while preserving the schema-v3 result model and deterministic
  output.
- Simplify standalone Vite application detection to require a direct Vite
  dependency and same-root `index.html`. Vite configuration and build scripts
  remain optional facts, and Vite-powered libraries do not become
  applications.
- Defer React Router Framework Mode application detection until its deployment
  requirements are defined more precisely.

0.5.0 - 2026-07-28
------------------

Kenbun can now analyze remote repositories incrementally with the same
deterministic application model used for local scans.

- Add stateful `remote_analysis()` and the pure `analyze()` primitive. Callers
  provide a path inventory, fetch only ordered `FileRequest` entries, and
  receive bounded partial results when content is unavailable.
- Add configurable per-file and per-session limits, manifest-first remote
  probing, and safe handling for oversized files and Git LFS pointers,
  including ignore files.
- Move the public result to schema v3 with explicit analysis status,
  completeness, and pending file requests. Resolved dependency and lockfile
  content facts are removed.
- Infer Node package-manager names from explicit metadata or lockfile paths
  without fetching lockfile contents, report explicit package-manager versions,
  and diagnose ambiguous manager evidence.

0.4.0 - 2026-07-10
------------------

Kenbun now discovers applications across Python and modern JavaScript and
TypeScript repositories more reliably, with bounded parsing and clearer
runtime metadata.

- Prevent recursive requirements includes and deeply nested Node packages from
  causing runaway scan time or memory use.
- Fix workspace-relative application and entrypoint hints, nested application
  boundaries, requirements grouping, pnpm YAML parsing, workspace brace globs,
  and strict Vite build-command detection.
- Parse PEP 723 script metadata, legacy PDM dependencies, exact PEP 735 group
  references, and absolute FastAPI re-exports while rejecting nameless URL and
  VCS requirement lines.
- Report inherited Python versions from `.python-version` and `.tool-versions`,
  plus Node versions from `.node-version`, `.nvmrc`, `.tool-versions`, and
  `package.json#engines.node`.
- Surface invalid roots, unreadable or oversized metadata, filesystem walk
  failures, and non-UTF-8 paths as diagnostics; keep followed symlinks inside
  the scan root and make `.ignore` handling explicit.
- Gate Rust tests and dependency policy in CI, test free-threaded CPython and
  PyPy, publish only the exact revision whose artifacts passed CI, and bundle
  third-party license notices in wheels and source distributions.

0.3.0 - 2026-07-10
------------------

Kenbun now discovers applications across Python and modern JavaScript and
TypeScript repositories, including monorepos.

This release introduces the schema-v1 `ScanResult.applications` model with
normalized technologies, dependency sets, build scripts, entrypoints,
evidence, and diagnostics. It replaces the previous
project/deploy-target/classification response, and renames the `scan()`
directory hint from `target_dir` to `application_dir`.

- Detect FastAPI, Django, Flask, Next.js, Astro, Nuxt, SvelteKit, TanStack
  Start, React Router Framework Mode, SolidStart, legacy Remix, and strict
  standalone Vite applications.
- Report JavaScript, TypeScript, React, Vue, Svelte, Solid, Vite, and Cross
  Inertia as normalized supporting technologies when their evidence belongs
  to the same application root.
- Discover uv, npm, pnpm, Yarn, Bun, and mixed workspaces, including upward
  discovery when scanning from inside a workspace member.
- Keep nested application boundaries isolated and use conservative
  package-manager and build-command inference.
- Add a network-independent unit suite plus an optional acceptance runner for
  29 scenarios pinned to immutable GitHub commits.

# CHANGELOG

## 0.2.3 - 2026-07-09

Verifies the GitHub release object is created automatically now that the
release runs autopub 1.0.0a60 (workflow-run event support). No library changes
since 0.2.2.

## 0.2.2 - 2026-07-09

Verifies the release pipeline end-to-end from a clean state, with all four
automation fixes in place (Windows ARM build, branch checkout for the Git push,
and idempotent publish). No library changes since 0.2.1.

## 0.2.1

Release-pipeline verification and fixes (no library changes since 0.2.0).

- Wheel builds no longer run autopub on every platform; the release version
  is computed once and stamped with a tomlkit-only script, so Windows ARM
  (which has no cryptography wheel) builds cleanly.
- The release job checks out the branch (not a detached HEAD) so the tag,
  changelog, and GitHub release are created automatically; publishing is
  idempotent via `uv publish --check-url`.

## 0.2.0

First working release of Kenbun.

The `0.1.0` version was taken by an unrelated placeholder upload, so the first
real release is `0.2.0`.

`kenbun.scan(path)` statically analyzes a directory without importing user
code and reports the deployable FastAPI applications it finds, their
entrypoints, dependencies, and diagnostics as typed objects with stable JSON.

- FastAPI detection across pyproject (PEP 621, optional dependencies, PEP 735
  groups, and Poetry), requirements.txt, Pipfile, and uv/pylock lockfiles,
  using PEP 503-normalized name matching.
- Static entrypoint resolution mirroring `fastapi run`: FastAPI CLI search
  order, `app`/`api` precedence, factories, `app = create_app()`, and one-hop
  re-exports without code execution.
- uv workspace discovery, including upward resolution from a member directory.
- Stable machine-readable diagnostics for missing applications, syntax errors,
  uninstallable dependencies, and invalid configured directories.
- `extra_ignore_files` so callers can analyze the same file set they upload.