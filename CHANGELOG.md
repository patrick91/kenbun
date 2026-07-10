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
