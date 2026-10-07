# HarfBuzz subset C API for HarfRust

A separate C library backed by fontations' `skera`, sharing `hr_face_t`,
`hr_blob_t`, and `hr_set_t` handles with the HarfRust shaping C library.
It exports `hr_subset_*`, with source aliases in `hr-hb-subset.h`.
HarfBuzz's opaque handles cannot be passed to these functions.

The initial API covers Skia's PDF workflow: create an input, select glyphs,
set `RETAIN_GIDS | NOTDEF_OUTLINE`, subset a face, and reference its output
blob. Unicode selection, reference-counted inputs, and `NO_HINTING` are also
supported. Input sets start empty and are borrowed from their input;
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

## Current scope

The dependency is the published Skera 0.8 release series. CFF, CFF2, and VARC
fonts return `NULL`: this release does not rewrite those outline tables.
Unsupported flag bits, missing enumeration, malformed inputs, and Skera
errors also return `NULL`, allowing Skia to embed the full font instead.
This is an initial API, not the complete HarfBuzz subset API: configurable
table/layout/name sets, subset plans, axis pinning, and the remaining flags
are not exposed yet. As with the shaping C API, allocation failure aborts.

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
