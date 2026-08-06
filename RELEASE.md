---
release type: patch
---

This release makes Python framework detection easier to extend without changing
analysis behavior.

- Give Django, FastAPI, and Flask dedicated detector modules with explicit,
  statically dispatched calls from the framework orchestrator.
- Share sorted dependency lookup between detectors and keep FastAPI entrypoint
  resolution within the FastAPI module.
