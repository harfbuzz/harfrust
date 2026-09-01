# harfrust-capi

A C API for [HarfRust](https://github.com/harfbuzz/harfrust), mirroring the
shaping half of HarfBuzz's API with an `hr_` prefix in place of `hb_`.

Every type, function, enumerator and constant is named and numbered to match
its HarfBuzz counterpart, so C code can usually be ported by renaming `hb_` to
`hr_`. The prefix also means this library can be linked into the same process
as HarfBuzz itself without collisions.

```c
#include <hr.h>

hr_blob_t   *blob   = hr_blob_create_from_file("font.ttf");
hr_face_t   *face   = hr_face_create(blob, 0);
hr_font_t   *font   = hr_font_create(face);
hr_buffer_t *buffer = hr_buffer_create();

hr_buffer_add_utf8(buffer, "Hello", -1, 0, -1);
hr_buffer_guess_segment_properties(buffer);
hr_shape(font, buffer, NULL, 0);

unsigned int count;
hr_glyph_info_t     *infos     = hr_buffer_get_glyph_infos(buffer, &count);
hr_glyph_position_t *positions = hr_buffer_get_glyph_positions(buffer, &count);
```

See [`examples/shape.c`](examples/shape.c) for a complete program.

## Drop-in replacement for HarfBuzz

[`include/hr-hb.h`](include/hr-hb.h) maps every HarfBuzz name onto its
HarfRust counterpart. Include it in place of `<hb.h>` and existing HarfBuzz
shaping code builds unchanged:

```c
#include <hr-hb.h>

hb_blob_t   *blob   = hb_blob_create_from_file("font.ttf");
hb_face_t   *face   = hb_face_create(blob, 0);
hb_font_t   *font   = hb_font_create(face);
hb_buffer_t *buffer = hb_buffer_create();

hb_buffer_add_utf8(buffer, "Hello", -1, 0, -1);
hb_buffer_guess_segment_properties(buffer);
hb_shape(font, buffer, NULL, 0);
```

[`examples/hb-compat.c`](examples/hb-compat.c) is the same program as
`shape.c` written entirely in HarfBuzz's names, and mentions HarfRust nowhere.

Two things to know. It cannot be combined with HarfBuzz itself in one
translation unit, since the macros would rewrite HarfBuzz's own declarations;
include one or the other. And it covers only the shaping API, so anything from
the list below fails to compile rather than failing at run time, which is the
point.

The header is generated from `hr.h`, so the two cannot drift:

```sh
python3 scripts/gen-hb-compat-header.py
```

## Building

```sh
cargo build -p harfrust-capi --release
```

This produces a static library, a shared library and a Rust `rlib`. The header
is committed at [`include/hr.h`](include/hr.h) and is generated with
[cbindgen](https://github.com/mozilla/cbindgen):

```sh
cbindgen --config harfrust-capi/cbindgen.toml \
         --crate harfrust-capi \
         --output harfrust-capi/include/hr.h
```

Regenerate it after changing any `pub extern "C"` item.

## Object lifetime

Objects are reference counted. Constructors return a new reference the caller
owns; `hr_*_reference` takes another, and `hr_*_destroy` gives one back.

`hr_*_create` never returns `NULL`: on failure it returns an immortal empty
object, and the `_or_fail` variants return `NULL` instead. Getters accept
`NULL` too, behaving as though passed the empty object. As in HarfBuzz this
means a chain of calls can be written without checking every result, and the
error surfaces as an empty shaping result rather than a crash.

Objects can carry `user_data` keyed by the address of an `hr_user_data_key_t`,
and can be frozen with `hr_*_make_immutable`, after which setters are ignored.

## What is covered

Blobs, faces, fonts, font callbacks, buffers, shape plans and `hr_shape`,
along with the tags, directions, scripts, languages, features and variations
they need.

Faces can be built two ways: over a blob with `hr_face_create`, or from a
callback with `hr_face_create_for_tables`, which asks for one table at a time.

`hr_shape` already reuses shape plans through a per-face cache, the way
`hb_shape` does internally, so reach for `hr_shape_plan_create` only when you
want to hold a plan yourself. `hr_shape_plan_create_cached` draws from that
same cache.

## What is not

HarfRust is a shaping library, so anything outside shaping is absent:

- Drawing and painting callbacks (`hb_draw_funcs_t`, `hb_paint_funcs_t`).
- Subsetting.
- Layout table introspection (`hb_ot_layout_*`), and the `hb_set` / `hb_map`
  containers it reports through.
- Custom Unicode callbacks (`hb_unicode_funcs_t`); HarfRust's own Unicode data
  is always used.
- `hb_buffer_diff`, buffer message callbacks, and `hb_font_get_glyph_name`.
- The buffer's replacement codepoint (`hb_buffer_set_replacement_codepoint`);
  invalid UTF is always replaced with U+FFFD, which is HarfBuzz's default.

## Deliberate differences from HarfBuzz

- **Table callbacks must be thread safe.** `hr_face_create_for_tables` may
  invoke its callback from whichever thread first touches a given table,
  because that is when the underlying library loads it. HarfBuzz makes no such
  demand. The same applies to font callbacks and to every `destroy` function.

- **Blobs are read-only.** `hr_blob_get_data_writable` always returns `NULL`;
  shaping never needs to modify font data in place. Use
  `hr_blob_copy_writable_or_fail` for a private, mutable copy.

- **Flag types, `hr_script_t` and `hr_direction_t` are integer typedefs**
  rather than enums. C combines flags with a bitwise or, and fills a
  `hr_segment_properties_t` in itself, so neither can be held in a Rust
  enumeration without inviting undefined behaviour on values it does not list.
  The constants carry the same values as HarfBuzz's enumerators and still work
  as `switch` case labels.

- **Misusing the shaping calls aborts,** as HarfBuzz's assertions do, rather
  than being reported. `hr_shape` and `hr_shape_full` abort when handed a
  buffer that already holds glyphs or a font with nothing to shape with, and
  `hr_shape_plan_execute` aborts on a plan built for another face, other
  variation settings, or properties the buffer does not carry. HarfBuzz
  compiles its assertions out with `NDEBUG`; these are always on, because
  `hr_shape` returns nothing and could not otherwise report them at all.

  Running past the length, operation or nesting limits is not in that set.
  Pathological input can provoke it, so `hr_shape_full` returns false and
  `hr_shape` carries on, exactly as HarfBuzz does.

- **Only the text serialization format** is supported by
  `hr_buffer_serialize_glyphs`; asking for JSON serializes nothing.

- **The plan cache is bounded** at 32 entries per face, where HarfBuzz's list
  is unbounded.

- **`hr_shape` fills in unset segment properties** rather than failing, since
  building a plan requires a direction.

## Threading

Faces and fonts may be shared between threads; the plan cache and every
reference count are synchronised internally. A single buffer must not be used
from two threads at once, which matches HarfBuzz.

## License

MIT, the same as the rest of the project.
