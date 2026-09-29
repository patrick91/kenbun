---
release type: patch
---

This release stops unrelated large Python files from making remote scans partial.

- A root-level script that remote analysis would never request, such as a
  large generated `pages.py`, no longer marks the result `partial` when it
  exceeds `max_file_bytes`. Oversized files the analysis needs are still
  reported as unavailable.
