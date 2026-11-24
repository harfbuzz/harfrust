# The WebAssembly shaper in HarfRust

HarfBuzz has an optional shaper that runs a font's own shaping engine from a
`Wasm` table. HarfRust ports it behind the `wasm` feature.

```toml
harfrust = { version = "0.16", features = ["wasm"] }
```

When the feature is enabled, [`shape`](../harfrust/src/shape.rs) uses the
embedded module instead of the OpenType shaper for fonts whose `Wasm` table can
be read. Passing an explicit `ShapePlan` selects the OpenType shaper, which is
how the test suite keeps deterministic OpenType results for fonts that also
carry a `Wasm` table.

## Embedding a WebAssembly host

The host needs to run untrusted `wasm32` code, call back into the shaper, and
still build on constrained targets. In particular, a HarfRust build with the
feature enabled must itself compile to `wasm32-unknown-unknown`.

| Host | Pure Rust | `no_std` | Runs on `wasm32` | Notes |
| - | - | - | - | - |
| [`wasmi`](https://crates.io/crates/wasmi) | yes | yes | yes | Interpreter, fuel metering, deterministic, audited. **Chosen.** |
| `wasmtime` | no | no | no | JIT, native only. |
| `wasmer` | no | no | no | JIT, native only. |
| `wasm-micro-runtime` | no | yes | no | C library used by HarfBuzz; adds a C toolchain and an interpreter/JIT per platform. |

Harfbuzz's protocol passes only `i32` tokens and pointers, so reference types
are not needed. `wasmi` is used with `default-features = false` and
`prefer-btree-collections`, which avoids a random source requirement and keeps
the build `no_std`-friendly.

The module is compiled once per font and cached in the shared shaping cache
(`LayoutCache`). A fresh `Store` and module instance are created for every
shaping call, so guest state cannot leak between runs. Guest linear memory is
accessed through `Memory::read`/`write`, and the host-side bump allocator places
its allocations above the memory the guest has already grown.

Verify self-hosting with:

```sh
cargo check -p harfrust --no-default-features --features libm,wasm \
    --target wasm32-unknown-unknown
```

## Host API

The module may export `shape_plan_create`, `shape` and `shape_plan_destroy`,
and imports the standard HarfBuzz host functions from the `env` module:

- common: `script_get_horizontal_direction`
- buffer: `buffer_copy_contents`, `buffer_set_contents`,
  `buffer_contents_realloc`, `buffer_contents_free`, `buffer_get_direction`,
  `buffer_get_script`, `buffer_reverse`, `buffer_reverse_clusters`
- face: `face_create`, `face_copy_table`, `face_get_upem`
- font: `font_create`, `font_get_face`, `font_get_scale`, `font_get_glyph`,
  `font_get_glyph_h_advance`, `font_get_glyph_v_advance`,
  `font_get_glyph_extents`, `font_glyph_to_string`, `font_copy_coords`,
  `font_set_coords`, `font_copy_glyph_outline`
- outline/blob: `glyph_outline_free`, `blob_free`
- shaping: `shape_with` (the `ot` shaper), `feature_copy_contents`,
  `features_free`
- debug: `debugprint` through `debugprint4`

Unknown function imports (for example wasm-bindgen helpers) are bound to
zero-returning stubs so that modules which never call them still instantiate.

## Differences from the HarfBuzz host

- `font_set_coords` works for fonts created by the guest. The root font is
  borrowed immutably, so setting its coordinates reports failure; create a face
  and a font from it instead.
- `shape_with` supports the `ot` shaper. HarfBuzz can also delegate to other
  compiled-in shapers (for example Graphite); HarfRust has no such shaper.
- A `Wasm` table is discovered for SFNT fonts backed by a blob. Fonts built
  from a read-fonts `TableFunction` cannot expose arbitrary tables yet, so they
  fall back to OpenType shaping.
- If the module runs but `shape` traps or returns zero, shaping returns
  `ShapeError::WasmShapingFailed`. HarfBuzz retries once before giving up.
- `font_copy_glyph_outline` draws through read-fonts' outline support and
  ignores custom `FontFuncs` draw callbacks, which HarfRust does not have.

## Testing

`harfrust/tests/wasm.rs` installs hand-written `.wat` modules into a normal
font and checks:

- codepoint-to-glyph mapping and default advances,
- `shape_with("ot")` followed by buffer post-processing,
- face, table, glyph name, extents, variation and outline queries.

The same host was also exercised with a real `wasm-bindgen` guest built from
HarfBuzz's `harfbuzz-wasm` crate.
