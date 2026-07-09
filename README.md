# kenbun

`kenbun` (見聞 — "seeing and hearing") is a static-analysis library that
identifies what kind of Python project lives inside a directory: which
projects exist, whether they are web apps or libraries, which framework they
use, and where their entrypoint is — **without ever importing user code**.

It is designed for tools that need to inspect a codebase and decide what to
do with it, such as a cloud deploy CLI, a project setup wizard, or a repo
analysis service. It understands uv workspaces and is built to also analyze
partial file sets (for example, a few files fetched from the GitHub API)
rather than only directories on disk.

The full design is in [`docs/spec.md`](docs/spec.md).

> **Status:** early development. The detection engine described in the spec
> is being built; the current package only exposes a placeholder API.

## Development

Build the extension and run the tests with:

```bash
uv run maturin develop --uv
uv run pytest
```
