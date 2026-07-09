# CHANGELOG

## 0.2.1

Release-pipeline verification and fixes (no library changes since 0.2.0).

- Wheel builds no longer run autopub on every platform; the release version
  is computed once and stamped with a tomlkit-only script, so windows-arm
  (which has no cryptography wheel) builds cleanly.
- Release job checks out the branch (not a detached HEAD) so the tag,
  changelog, and GitHub release are created automatically; publish is
  idempotent via `uv publish --check-url`.

## 0.2.0

First working release of kenbun.

(The `0.1.0` version was taken by an unrelated placeholder upload, so this
first real release is `0.2.0`.)

`kenbun.scan(path)` statically analyzes a directory — without importing user
code — and reports the deployable FastAPI apps it finds, their entrypoints,
dependencies, and any problems, as typed objects with a stable JSON form.

- FastAPI detection across pyproject (PEP 621 / optional-deps / PEP 735
  groups / Poetry), requirements.txt, Pipfile, and uv/pylock lockfiles, with
  PEP 503-normalized name matching.
- Static entrypoint resolution mirroring `fastapi run` (ruff parser): the
  fastapi-cli search order and `app`/`api` precedence, factories,
  `app = create_app()`, and one-hop re-exports — no code execution.
- uv workspace discovery, including upward resolution from a member directory.
- Stable, machine-readable diagnostics (KB codes) for the common deploy
  problems: no app, syntax errors, uninstallable dependencies, bad configured
  directory.
- `extra_ignore_files` so callers can honor their own ignore files (e.g.
  `.fastapicloudignore`) and analyze exactly the set of files that ship.
