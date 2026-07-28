---
release type: minor
---

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
