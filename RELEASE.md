---
release type: minor
---

Exhausting a remote analysis's round budget no longer throws away what it found.

- `max_rounds` now ends the session instead of raising. `file_requests` becomes
  empty, the new `round_limit_reached` property becomes `True`, and `result`
  reports `completeness="partial"`. This brings it in line with `max_files` and
  `max_file_bytes`, which have always narrowed a result rather than failing it.
- Callers previously had to catch a bare `RuntimeError` and re-run `analyze()`
  to recover the facts the session already held. `result` is now reachable
  directly, so a bounded analysis costs nothing extra and cannot disagree with
  the session it came from.
- A `RuntimeError` from `update()` consequently always signals a broken
  invariant rather than an exhausted budget, so callers should no longer catch
  it. The result's `status` still reports `needs_files` when the analysis wanted
  more content; the session's decision to stop is reported separately by
  `round_limit_reached`.
