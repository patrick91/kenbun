---
release type: minor
---

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
