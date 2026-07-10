# Third-party software notices

Kenbun statically links Rust dependencies into its extension module. The
dependency set and license expressions are checked from `Cargo.lock` by
`cargo deny`, and this notice is included in both wheels and source
distributions.

The complete locked component inventory can be reproduced with:

```console
cargo metadata --locked --format-version 1
```

Most components are offered under MIT, Apache-2.0, or a choice of those
licenses. Their authors retain their respective copyrights. The following
locked components require additional notice because their declared license
set is not solely MIT-compatible:

| Components | Version | License expression |
|---|---:|---|
| `icu_collections`, `icu_locale_core`, `icu_normalizer`, `icu_normalizer_data`, `icu_properties`, `icu_properties_data`, `icu_provider` | 2.2.0 | Unicode-3.0 |
| `litemap` | 0.8.2 | Unicode-3.0 |
| `potential_utf` | 0.1.5 | Unicode-3.0 |
| `tinystr` | 0.8.3 | Unicode-3.0 |
| `writeable` | 0.6.3 | Unicode-3.0 |
| `yoke` | 0.8.3 | Unicode-3.0 |
| `yoke-derive` | 0.8.2 | Unicode-3.0 |
| `zerofrom` | 0.1.8 | Unicode-3.0 |
| `zerofrom-derive` | 0.1.7 | Unicode-3.0 |
| `zerotrie` | 0.2.4 | Unicode-3.0 |
| `zerovec` | 0.11.6 | Unicode-3.0 |
| `zerovec-derive` | 0.11.3 | Unicode-3.0 |
| `unicode-ident` | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `unicode_names2` | 1.3.0 | (MIT OR Apache-2.0) AND Unicode-DFS-2016 |
| `version-ranges` | 0.1.3 | MPL-2.0 |
| `pep440_rs` | 0.7.3 | Apache-2.0 OR BSD-2-Clause |
| `pep508_rs` | 0.9.2 | Apache-2.0 OR BSD-2-Clause |
| `ryu` | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| `target-lexicon` | 0.13.5 | Apache-2.0 WITH LLVM-exception |
| `tinyvec` | 1.11.0 | Zlib OR Apache-2.0 OR MIT |
| `zerocopy`, `zerocopy-derive` | 0.8.54 | BSD-2-Clause OR Apache-2.0 OR MIT |

The full license terms are available in each crate's packaged `LICENSE*` or
`COPYING*` file in the Cargo registry source archive. SPDX license texts are
also available from <https://spdx.org/licenses/>. This inventory is generated
from the locked dependency metadata and must be updated whenever `Cargo.lock`
changes.
