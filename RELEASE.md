---
release type: patch
---

This release accepts `pnpm-workspace.yaml` files that only hold pnpm settings.

- pnpm 10 writes settings such as `onlyBuiltDependencies` to
  `pnpm-workspace.yaml` even in repositories without workspace packages. A
  file without `packages` now declares no members, like `packages: ["."]`,
  instead of reporting `KB203` and making remote results `partial`.
