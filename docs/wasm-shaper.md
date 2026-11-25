# The WebAssembly shaper in HarfRust

HarfBuzz has an optional shaper that runs a font's own shaping engine from a
`Wasm` table. HarfRust ports it behind the `wasm` feature.

```toml
harfrust = { version = "0.16", features = ["wasm"] }
```

When the feature is enabled, a font's embedded module is used when it is
asked for. The shaper is chosen by [`ShapeOptions::shapers`], an ordered list
that defaults to `[Shaper::Ot]`; pass `[Shaper::Wasm, Shaper::Ot]` to run the
module of a font that has one and leave the rest to OpenType, as HarfBuzz
does. This keeps the OpenType shaper the default for callers who never asked
for the feature, and makes running a module an explicit choice rather than a
side effect of caching plans.

```rust
let options = ShapeOptions::new().shapers(&[Shaper::Wasm, Shaper::Ot]);
```

The `hr-shape` tool exposes the same choice: `--shapers=wasm,ot` tries the
module first and leaves the rest to OpenType, `--shapers=wasm` alone insists on
it, and `--list-shapers` prints the shapers the build supports.

A supplied `ShapePlan` records the shaper it was built for, and that shaper is
used instead of the list.

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
- face: `face_create`, `face_copy_table`, `face_get_upem`, `face_destroy`
- font: `font_create`, `font_destroy`, `font_get_face`, `font_get_scale`,
  `font_get_glyph`, `font_get_glyph_h_advance`, `font_get_glyph_v_advance`,
  `font_get_glyph_extents`, `font_glyph_to_string`, `font_copy_coords`,
  `font_set_coords`, `font_copy_glyph_outline`
- outline/blob/coords: `glyph_outline_free`, `blob_free`, `coords_free`
- shaping: `shape_with` (the `ot` shaper), `feature_copy_contents`,
  `features_free`
- debug: `debugprint` through `debugprint4`

Unknown function imports (for example wasm-bindgen helpers) are bound to
zero-returning stubs so that modules which never call them still instantiate.

The copy functions — `buffer_copy_contents`, `face_copy_table`,
`font_copy_coords` and `font_copy_glyph_outline` — write back how much they
copied. A module can pass the size of its arrays and read back what was used,
as HarfBuzz does.

## Limits

A font's module is not trusted. Each `shape` call makes a new instance of it,
and that instance gets:

- **Fuel.** 2^27, plus 3 × 2^21 for each item in the buffer. wasmi takes one
  unit for each of its instructions, and one for every 64 bytes that growing
  or copying memory touches. Host functions charge the same account for the
  work they do for the module: 64 for the call, and more where the work grows
  with a size, such as one for every eight bytes copied, 4096 for a glyph
  drawn plus 64 for each of its points, and 4096 for each item `shape_with`
  shapes.
- **Memory.** 32 MiB, plus twice the size of the font, plus 128 times the
  size of the buffer's contents. The data of the faces and fonts that the
  module creates, which the host holds, counts against the same allowance,
  and a module can create 256 of them.
- **Buffer length.** The per-text limits that HarfBuzz installs around every
  shaper: 512 items for each item the module was given, and never fewer than
  65536. `buffer_set_contents` refuses a longer buffer. `shape_with` puts the
  limits back afterwards, because the OpenType shaper it delegates to resets
  them on its way out.

A module that runs out of fuel traps, and `shape` returns
`ShapeError::WasmShapingFailed`. A request for more memory than the allowance
is refused. A module that cannot be instantiated within these limits, for
example because its start function does not return, leaves the font to the
OpenType shaper, as any module that fails to instantiate does.

The numbers are those of HarfBuzz's WebAssembly shaper (`hb-limits.hh`), with
the fuel per item scaled by one and a half: wasmi takes 1.1 to 1.4 times as
much fuel as wasm-micro-runtime counts instructions for the same module.

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
- `face_destroy` and `font_destroy` free a face or font the guest created,
  and its token names nothing afterwards. In HarfBuzz they also let the
  module instance be kept for the next call; HarfRust creates an instance per
  call either way.
- `font_copy_glyph_outline` draws through read-fonts' outline support and
  ignores custom `FontFuncs` draw callbacks, which HarfRust does not have.
- HarfBuzz gives host work an allowance of its own, the size of its
  instruction limit; here it comes out of the same fuel. HarfBuzz cannot give
  a call more than 2^31 instructions, which its runtime counts in an `int`;
  there is no such ceiling here.
- HarfBuzz does not run a module that imports functions other than the host
  API, has a start function, or exports an allocator, because its runtime
  would run code for such a module that cannot be limited. None of that
  applies here: unknown imports get stubs, the start function runs on the
  call's fuel, and the host never runs the module's allocator.

## Testing

`harfrust/tests/wasm.rs` installs hand-written `.wat` modules into a normal
font and checks:

- codepoint-to-glyph mapping and default advances,
- `shape_with("ot")` followed by buffer post-processing,
- the shaper list: OpenType by default even for a font with a module, falling
  through to the next shaper when a font has no module, and
  `ShapeError::ShaperUnavailable` when no listed shaper can shape the font,
- face, table, glyph name, extents, variation and outline queries,
- features passed to the module, right-to-left input, `buffer_contents_realloc`,
  `font_set_coords` on a font the module created, and the refusals that leave a
  font to OpenType: no `shape` export, no memory, and an empty buffer,
- the limits: a module that loops, one that takes memory until refused, one
  that has the host copy a table a hundred thousand times, one that creates
  fonts until refused, one that asks the buffer for more items than the call
  allows, and one that asks again after `shape_with`.

`harfrust/tests/fonts/wasm` holds three of HarfBuzz's test fonts, whose modules
are compiled from C against `hb-wasm-api.h`: one imports a function nothing
provides and never calls it, one answers with values chosen by the first
character and is used here for a `shape` that returns false, and one copies and
frees the font's coordinates two hundred thousand times. Those exercise the same
host API through a real toolchain's output rather than hand-written `.wat`.

The same host was also exercised with a real `wasm-bindgen` guest built from
HarfBuzz's `harfbuzz-wasm` crate.
