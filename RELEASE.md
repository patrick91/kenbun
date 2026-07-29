---
release type: minor
---

Kenbun can now focus repository analysis on Python or Node ecosystems
independently.

- Add the optional `ecosystems` argument to `scan()`, `analyze()`, and
  `remote_analysis()`, with typed support for selecting Python, Node, or both.
  Disabled detectors no longer contribute workspace framing, file requests,
  facts, or diagnostics.
- Group Python and JavaScript/TypeScript detection under explicit ecosystem
  modules while preserving the schema-v3 result model and deterministic
  output.
- Simplify standalone Vite application detection to require a direct Vite
  dependency and same-root `index.html`. Vite configuration and build scripts
  remain optional facts, and Vite-powered libraries do not become
  applications.
- Defer React Router Framework Mode application detection until its deployment
  requirements are defined more precisely.
