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
      +-----+----------------------+
      |                            |
python::discover()           node::discover()
      |                            |
Python projects               raw Node facts
      |                       /            \
      +----------------------+              \
                 |                            |
      assembly::applications()    workspace::discover_at_root()
                 |                            |
          applications                    workspace
                 +-------------+--------------+
                               |
                          ScanResult
```

`FileSet` is the shared input abstraction. Local scans populate it by walking a
filesystem; remote analysis populates it from an inventory and requested
contents. Detectors must use `FileSet` rather than reading repository paths
directly so both analysis modes retain the same behavior.

## Module ownership

- `lib.rs` is the PyO3 boundary. It validates Python arguments, releases the
  GIL, and delegates to the Rust analysis pipeline.
- `scan.rs` owns the analysis lifecycle: input framing, hint validation,
  detector invocation, completeness, and final diagnostics.
- `python.rs` owns Python project discovery and Python-specific candidate
  types. `python/manifest.rs` parses dependency metadata and
  `python/entrypoint.rs` performs static FastAPI entrypoint resolution.
- `node.rs` coordinates JavaScript/TypeScript discovery.
  `node/manifest.rs`, `node/workspace.rs`, and `node/command.rs` own their
  respective parsing concerns.
- `assembly.rs` is the only place that combines ecosystem facts into public
  `Application` values. It preserves same-directory enrichment and applies
  cross-ecosystem rules such as Cross Inertia.
- `workspace.rs` owns upward workspace framing and reconciliation of uv and
  Node workspace facts.
- `boundary.rs` owns the shared set of Python and Node project-boundary
  markers used to prevent nested source evidence from leaking into a parent.
- `model.rs` contains only the public output schema. Detector-private
  candidates belong to their detector modules.

## Invariants

- Python and Node detection run independently; neither ecosystem is the base
  representation for the other.
- One public application is emitted per `application_dir`.
- A non-primary contribution may enrich a primary contribution at the same
  directory. For example, a Python library can contribute dependency and
  runtime facts to a Node application.
- Cross-ecosystem rules belong in the assembler, not in either detector.
- Public schema shape, canonical ordering, diagnostics, and remote file-request
  behavior must remain stable during internal refactors.

## Deliberate non-goals

Kenbun does not currently need a generic ecosystem trait, dynamic registry, or
third-party detector API. Introduce such an abstraction only when another
ecosystem is being implemented and its concrete requirements are known.
