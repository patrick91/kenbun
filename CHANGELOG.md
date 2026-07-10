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

## Unreleased

- Replace the Python-project/deploy-target/classification response with the
  schema-v1 `ScanResult.applications` model. Applications expose normalized
  technologies, ecosystem-specific dependency sets, explicit build-script
  facts, entrypoints, evidence, and diagnostics.
- Rename the `scan()` directory hint from `target_dir` to `application_dir`.
  Kenbun remains a filesystem-only static analyzer; command selection,
  recommendations, and deployability policy belong to consumers.
- Add application detection for Django and Flask identity, Next.js, Astro,
  Nuxt, SvelteKit, TanStack Start, React Router Framework Mode, SolidStart,
  legacy Remix, and strict standalone Vite applications while preserving the
  detailed FastAPI resolver.
- Add same-root supporting technology detection for JavaScript/TypeScript,
  React, Vue, Svelte, Solid, Vite, and normalized `cross-inertia` integration.
  Nested package evidence remains isolated unless the nested directory
  independently qualifies as an application.
- Add uv, npm, pnpm, Yarn, Bun, and mixed-workspace discovery, including
  upward discovery from directories inside members, explicit Node build-script
  facts, and conservative package-manager inference with no implicit npm default.
- Add a manual external-fixture runner backed by full, immutable GitHub commit
  SHAs. The regular test suite remains network-independent.

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