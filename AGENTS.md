# AGENTS.md

These instructions apply to the whole repository.

## Project contract

Kenbun is a Rust-backed Python library for deterministic, static repository
analysis. It supports Python and modern JavaScript/TypeScript. Treat every
scanned repository as untrusted input.

- Never import or execute repository modules, install their dependencies, run
  package scripts, or evaluate configuration files.
- Do not add network or subprocess access to the analysis pipeline.
- Keep reads bounded and preserve the configured file-count and file-size
  limits.
- Parse failures should become stable diagnostics and partial facts where
  possible; do not panic on repository content.
- Preserve deterministic ordering and byte-identical JSON for unchanged
  inputs.
- Do not add `unsafe` code without explicit approval and a documented safety
  argument.

`docs/spec.md` is normative for public behavior. Update it when public APIs,
schema, diagnostics, or detection behavior change. Keep
`docs/architecture.md` aligned with module ownership and data flow.

## Architecture

- `src/lib.rs` is the PyO3 boundary. Validate Python values there before
  releasing the GIL.
- `src/scan.rs` orchestrates analysis and calls enabled ecosystem detectors.
- `src/python.rs` and `src/python/` own Python discovery and parsing.
- `src/node.rs` and `src/node/` own JavaScript/TypeScript discovery and parsing.
- `src/assembly.rs` is the only module that combines ecosystem facts into
  public applications.
- `src/workspace.rs` owns upward workspace framing and root reconciliation.
- `src/fileset.rs` is the shared local/remote input abstraction. Detectors must
  not bypass it.
- `src/model.rs` contains the public output schema, not detector-private
  candidates.
- `src/kenbun/` owns the typed Python API and stateful remote-analysis wrapper.

Keep Python and Node detection independent. Cross-ecosystem behavior belongs in
assembly, and support for a new ecosystem should be based on concrete
requirements rather than a speculative plugin abstraction.

## Paths and portability

- Public repository paths are normalized, repository-relative POSIX paths using
  `/`.
- Host filesystem boundaries use `Path`/`PathBuf` in Rust and `PathLike`/`Path`
  in Python.
- Reject absolute paths, parent traversal, and symlink escapes at the boundary.
- Avoid Unix-only behavior in shared analysis code. Isolate unavoidable
  platform-specific code behind a narrow `cfg` and test portable behavior.

## Rust

- Keep imports at the top and put the main public flow before supporting
  helpers ("newspaper order").
- Prefer small explicit data structures and concrete detector calls over
  hidden control flow.
- Document public or architectural types and non-obvious invariants concisely.
- Avoid stale comments that repeat the implementation.
- Do not silence lints broadly. If a suppression is genuinely necessary, use
  a narrow `#[expect(...)]` with a reason rather than `#[allow(...)]`.
- Prefer safe Rust and propagate recoverable errors or diagnostics.
- Keep PyO3 conversion and validation out of detector modules.

## Python

- Keep the public Python surface fully typed, including `_kenbun.pyi`.
- Accept general iterables and mappings where the public type promises them;
  materialize one-shot iterables when state must persist across rounds.
- Follow the repository's Ruff formatting and double-quote style.
- Use exact, actionable exceptions for invalid caller input.
- Prefer existing test modules and parameterize equivalent cases instead of
  multiplying near-identical tests.
- Do not use `xfail` to hide known failures.

The extension can be stale relative to Rust source. Rebuild it before trusting
Python test results after any Rust boundary or analysis change.

## Verification

Run the smallest relevant tests while iterating, then run the complete checks
for a finished Rust/Python change:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
uv run ruff check .
uv run ruff format --check .
uv run maturin develop --uv
uv run pytest -q
.venv/bin/python scripts/check_external_fixtures.py --offline
cargo deny check
```

The external fixture corpus is acceptance coverage, not a substitute for
focused unit tests.

## Change hygiene

- Keep diffs narrow and preserve unrelated user changes and untracked files.
- Do not commit, push, or alter a pull request unless the user explicitly asks
  for that action.
- Stage files explicitly; never sweep unrelated work into a commit.
- Update the changelog for user-visible behavior.
