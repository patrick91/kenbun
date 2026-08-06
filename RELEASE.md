---
release type: patch
---

This release makes Python and Node framework detection easier to extend without
changing analysis behavior.

- Give Django, FastAPI, and Flask dedicated detector modules with explicit,
  statically dispatched calls from the framework orchestrator.
- Give Astro, Next.js, Nuxt, Remix, SolidStart, SvelteKit, and TanStack Start
  the same explicit per-framework structure on the Node side.
- Share sorted dependency lookup between detectors and keep FastAPI entrypoint
  resolution within the FastAPI module.
