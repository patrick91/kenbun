---
release type: minor
---

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
