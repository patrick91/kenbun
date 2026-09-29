---
release type: patch
---

This release makes partial scan results explain which files they are missing.

- A root-level script that remote analysis would never request, such as a
  large generated `pages.py`, no longer marks the result `partial` when it
  exceeds `max_file_bytes`.
- Every file the analysis needs but cannot use (over the parse cap, a Git LFS
  pointer, undecodable, or not provided) is now reported as `KB801` with its
  path. Previously many of these made a result `partial` without any
  diagnostic.
