---
release type: minor
---

Remote repository analysis can now identify symbolic links without fetching
their link-target blobs as ordinary files.

- Add the optional `is_symlink` field to `FileEntry`. Marked entries are
  validated but excluded before `FileSet` construction, so a linked manifest,
  ignore file, lockfile, or source file cannot invent repository facts.
- Never request or follow remote symlinks. Pre-supplied link-target contents
  are ignored, symlinks do not consume file budgets, and deliberately excluding
  them does not reduce completeness. This matches local scans with
  `follow_symlinks=False`.
