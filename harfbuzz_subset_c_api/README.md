# HarfBuzz subset C API for HarfRust

A separate C library backed by fontations' `skera`, sharing `hr_face_t`,
`hr_blob_t`, and `hr_set_t` handles with the HarfRust shaping C library.
It exports `hr_subset_*`, with source aliases in `hr-hb-subset.h`.
HarfBuzz's opaque handles cannot be passed to these functions.

The initial API covers Skia's PDF workflow: create an input, select glyphs,
set `RETAIN_GIDS | NOTDEF_OUTLINE`, subset a face, and reference its output
blob. Unicode selection, reference-counted inputs, and `NO_HINTING` are also
supported. Configurable input sets cover names, language IDs, layout features
and scripts, and dropped tables. `hr_subset_input_keep_everything` configures
all selections, with subsequent customization allowed. Glyph and Unicode sets start empty and are borrowed from their input;
`hr_set_reference` keeps a set alive independently.

Blob, TTC, and table-callback faces are accepted. Callback faces must install
`hr_face_set_get_table_tags_func`. Input tables are copied into a standalone
SFNT before calling Skera, so the source and input can be destroyed immediately
after a successful call. Zero-length tables are accepted.

## Building and linking

```sh
cargo build -p harfrust_capi -p harfbuzz_subset_c_api
cc -std=c99 -Iharfrust_capi/include -Iharfbuzz_subset_c_api/include \
  harfbuzz_subset_c_api/examples/subset.c -Ltarget/debug \
  -lharfbuzz_subset_c -lharfrust_c -Wl,-rpath,"$PWD/target/debug" -o /tmp/hr-subset
/tmp/hr-subset harfrust/tests/fonts/rb_custom/PT_Sans-Caption-Web-Regular.ttf
```

Link both libraries. The subset crate deliberately imports the core C ABI
instead of linking another copy of the shaping crate into its output. Linux
and macOS shared builds resolve core symbols from the consumer. An explicit
shared-core dependency can be selected with `HARFRUST_C_LIB_DIR`; on Windows,
build `harfrust_capi` first and set that variable to its output directory
(containing its import library) when building this crate. Windows and macOS
runtime behavior has not been validated locally.

For static linking, put `libharfbuzz_subset_c.a` before `libharfrust_c.a` and
include the platform's Rust system libraries (on Linux: `-ldl -lpthread -lm`).

Regenerate headers with:

```sh
cbindgen --config harfbuzz_subset_c_api/cbindgen.toml \
  --crate harfbuzz_subset_c_api --output harfbuzz_subset_c_api/include/hr-subset.h
python3 scripts/gen-hb-compat-header.py
```

`hr_subset_preprocess` returns an immutable, self-contained copy of the face,
so its callbacks and original data can be released. It currently adds no
subsetting acceleration cache.

## Current scope

This extension temporarily pins Skera/write-fonts to the Plan API changes
in [Fontations #2234](https://github.com/googlefonts/fontations/pull/2234).
Restore published dependencies when that API is released. CFF, CFF2, and VARC
fonts return `NULL`: this release does not rewrite those outline tables.
Unsupported flag bits, missing enumeration, malformed inputs, and Skera
errors also return `NULL`, allowing Skia to embed the full font instead.
This is an initial API, not the complete HarfBuzz subset API. Axis pinning,
user-specified glyph renumbering, name overrides, and unimplemented flags are
not exposed yet. As with the shaping C API, allocation failure aborts.

## Validation

The C example verifies shared handles and a retained glyph after serialization.
The differential test requires `fontTools` and a HarfBuzz build with the subset
library and table-enumeration callback API:

```sh
python3 harfbuzz_subset_c_api/tests/compare_skia_hb.py \
  target/debug/libharfrust_c.so target/debug/libharfbuzz_subset_c.so \
  /path/to/libharfbuzz.so /path/to/libharfbuzz-subset.so
```

It compares blob, callback, and Unicode selections across five flag combinations
and three fonts, including a variable font, and checks mappings, advances,
outlines, retained holes, hinting, `.notdef`, input/output lifetimes, and failures.

Additional implemented flags are `NAME_LEGACY`, `SET_OVERLAPS_FLAG`,
`PASSTHROUGH_UNRECOGNIZED`, `GLYPH_NAMES`, `NO_PRUNE_UNICODE_RANGES`,
`NO_LAYOUT_CLOSURE`, and `NO_BIDI_CLOSURE`. Desubroutinization and IUP delta
optimization are not implemented by the dependency and remain unsupported.

`tests/compare_controls_hb.py` takes the same library arguments and checks
15 additional cases, input-set defaults, keep-everything, inverted sets,
custom table passthrough, overlap flags, and preprocessing ownership.

Subset plans snapshot input sets and font bytes. They can be executed repeatedly
after releasing the original face and input, and expose old-to-new, new-to-old,
and Unicode-to-original glyph maps. Returned maps are borrowed and must not
be mutated; `hr_map_reference` retains them independently of a plan.
Configurable no-subset table tags now pass through unchanged; explicit table
drops and hint removal take precedence.

`tests/compare_plans_hb.py` accepts the same library arguments. It compares
16 combinations of plan mappings and executes plans after changing/destroying
their inputs, retaining borrowed maps past plan destruction, and configuring
table passthrough.

Inputs and plans also support address-keyed user data. Metadata belongs to
the object only after a successful setter call; replacement and destruction
release it once, with callbacks invoked outside the lock for reentrancy.
The plan test compares these semantics with HarfBuzz and executes plans
concurrently.
