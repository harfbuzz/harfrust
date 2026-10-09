# HarfBuzz subset C API for HarfRust

The optional `subset` feature of `harfrust_capi` adds font subsetting backed
by fontations' `skera` to the same C library as shaping. It uses the existing
`hr_face_t`, `hr_blob_t`, and `hr_set_t` handles.
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
cargo build -p harfrust_capi --features subset
cc -std=c99 -Iharfrust_capi/include \
  harfrust_capi/examples/subset.c -Ltarget/debug \
  -lharfrust_c -Wl,-rpath,"$PWD/target/debug" -o /tmp/hr-subset
/tmp/hr-subset harfrust/tests/fonts/rb_custom/PT_Sans-Caption-Web-Regular.ttf
```

Link only `harfrust_c` for both shaping and subsetting. The `subset` feature
is disabled by default; default builds have no Skera or write-fonts dependency
and export no subset symbols. The subset headers remain available to consumers,
but using their functions requires a library built with `--features subset`.
The same feature applies to static and shared libraries on all platforms.

For static linking, use `libharfrust_c.a` and the platform's Rust system
libraries (on Linux: `-ldl -lpthread -lm`). Windows consumers link the matching
static library or DLL import library; no separate subset DLL is needed.

Regenerate headers with:

```sh
cbindgen --config harfrust_capi/cbindgen.toml \
  --crate harfrust_capi --output harfrust_capi/include/hr.h
cbindgen --config harfrust_capi/cbindgen-subset.toml \
  harfrust_capi/src/subset/mod.rs --output harfrust_capi/include/hr-subset.h
python3 scripts/gen-hb-compat-header.py
```

The subset header is generated separately from its module so `hr.h` stays
independent of optional APIs.

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
python3 harfrust_capi/tests/subset/compare_skia_hb.py \
  target/debug/libharfrust_c.so \
  /path/to/libharfbuzz.so /path/to/libharfbuzz-subset.so
```

It compares blob, callback, and Unicode selections across five flag combinations
and three fonts, including a variable font, and checks mappings, advances,
outlines, retained holes, hinting, `.notdef`, input/output lifetimes, and failures.
