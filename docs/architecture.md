# Architecture

Kenbun has two explicitly supported ecosystems: Python and modern
JavaScript/TypeScript. The implementation uses concrete detector calls for
those ecosystems rather than a detector registry or plugin trait.

## Analysis flow

```text
local path or remote inventory
            |
          FileSet
            |
      EcosystemSelection
            |
      +-----+----------------------+
      |                            |
ecosystems::python           ecosystems::node
      |                            |
Python projects               raw Node facts
      |                       /            \
      +----------------------+              \
                 |                            |
 ecosystems::assembly          ecosystems::workspace
                 |                            |
          applications                    workspace
                 +-------------+--------------+
                               |
                          ScanResult
```

`FileSet` is the shared input abstraction. Local scans populate it by walking a
filesystem; remote analysis populates it from an inventory and requested
contents. Detectors must use `FileSet` rather than reading repository paths
directly so both analysis modes retain the same behavior. Remote symlink
entries are validated and removed before `FileSet` construction, matching a
local scan with `follow_symlinks=False`.

## Module ownership

- `lib.rs` is the PyO3 boundary. It validates Python arguments, releases the
  GIL, and delegates to the Rust analysis pipeline.
- `scan.rs` owns the analysis lifecycle: input framing, hint validation,
  detector invocation, completeness, and final diagnostics.
- `ecosystems/` contains the concrete detectors and their shared
  reconciliation logic.
- `ecosystems/python.rs` owns Python project discovery and Python-specific
  candidate types. `ecosystems/python/manifest.rs` parses dependency metadata.
  `ecosystems/python/frameworks/mod.rs` explicitly invokes each Python
  framework detector and shares indexed dependency lookup between them.
  Framework-specific identity and resolution live in one module per framework;
  FastAPI's static entrypoint resolver is nested under its detector.
- `ecosystems/node.rs` coordinates JavaScript/TypeScript discovery.
  `ecosystems/node/manifest.rs` and `ecosystems/node/workspace.rs` own manifest
  and workspace parsing. `ecosystems/node/frameworks/mod.rs` explicitly invokes
  each primary-framework detector; package names and configuration-file
  conventions live in one module per framework. UI frameworks, Vite, and
  Inertia remain separate supporting technology classifiers.
- `ecosystems/assembly.rs` is the only place that combines ecosystem facts
  into public `Application` values. It preserves same-directory enrichment and
  applies cross-ecosystem rules such as Cross Inertia.
- `ecosystems/workspace.rs` owns upward workspace framing and reconciliation
  of uv and Node workspace facts.
- `ecosystems/runtime.rs` owns shared declarative Python and Node
  runtime-version facts.
- `ecosystems/boundary.rs` owns the shared set of Python and Node
  project-boundary markers used to prevent nested source evidence from leaking
  into a parent.
- `model.rs` contains only the public output schema. Detector-private
  candidates belong to their detector modules.

## Invariants

- Python and Node detection run independently; neither ecosystem is the base
  representation for the other.
- Ecosystem selection gates detector invocation, workspace discovery, and
  remote file requests at orchestration boundaries. Disabled detectors do not
  read their manifests or contribute facts and diagnostics.
- One public application is emitted per `application_dir`.
- A non-primary contribution may enrich a primary contribution at the same
  directory. For example, a Python library can contribute dependency and
  runtime facts to a Node application.
- Cross-ecosystem rules belong in the assembler, not in either detector.
- Public schema shape, canonical ordering, diagnostics, and remote file-request
  behavior must remain stable during internal refactors.
- Framework detector calls are concrete and ordered. Adding a framework means
  adding its module and an explicit call rather than registering dynamic
  dispatch or repeatedly scanning the complete dependency list.

## Deliberate non-goals

Kenbun does not currently need a generic ecosystem trait, dynamic registry, or
third-party detector API. Introduce such an abstraction only when another
ecosystem is being implemented and its concrete requirements are known.
