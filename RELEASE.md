---
release type: minor
---

Kenbun now discovers applications across Python and modern JavaScript and
TypeScript repositories, including monorepos.

This release introduces the schema-v1 `ScanResult.applications` model with
normalized technologies, dependency sets, build scripts, entrypoints,
evidence, and diagnostics. It replaces the previous
project/deploy-target/classification response, and renames the `scan()`
directory hint from `target_dir` to `application_dir`.

- Detect FastAPI, Django, Flask, Next.js, Astro, Nuxt, SvelteKit, TanStack
  Start, React Router Framework Mode, SolidStart, legacy Remix, and strict
  standalone Vite applications.
- Report JavaScript, TypeScript, React, Vue, Svelte, Solid, Vite, and Cross
  Inertia as normalized supporting technologies when their evidence belongs
  to the same application root.
- Discover uv, npm, pnpm, Yarn, Bun, and mixed workspaces, including upward
  discovery when scanning from inside a workspace member.
- Keep nested application boundaries isolated and use conservative
  package-manager and build-command inference.
- Add a network-independent unit suite plus an optional acceptance runner for
  29 scenarios pinned to immutable GitHub commits.
