/*!
A C API for [HarfRust](https://github.com/harfbuzz/harfrust), mirroring the
shaping half of HarfBuzz's API with an `hr_` prefix in place of `hb_`.

Every type, function, enumerator and constant is named and numbered to match
its HarfBuzz counterpart, so C code can usually be ported by renaming `hb_` to
`hr_`. The prefix also means this library can be linked into the same process
as HarfBuzz itself without collisions.

# Scope

This covers shaping only: blobs, faces, fonts, buffers and `hr_shape`. It has
no drawing or painting callbacks, no subsetting, no layout table introspection
and no `hb_set` / `hb_map` containers, because HarfRust does not provide them.

# Object lifetime

Objects are reference counted. Constructors return a new reference the caller
owns; `hr_*_reference` takes another, and `hr_*_destroy` gives one back. The
`hr_*_create` functions never return `NULL`: on failure they return an
immortal empty object, and the `_or_fail` variants return `NULL` instead. Every
getter also accepts `NULL`, behaving as though it were passed the empty object.
*/

// The whole point of this crate is to look like C.
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub mod blob;
pub mod buffer;
pub mod common;
pub mod face;
pub mod font;
pub mod font_funcs;
pub mod object;
mod plan;
pub mod shape;
pub mod shape_plan;

pub use blob::*;
pub use buffer::*;
pub use common::*;
pub use face::*;
pub use font::*;
pub use font_funcs::*;
pub use object::{hr_destroy_func_t, hr_user_data_key_t};
pub use shape::*;
pub use shape_plan::*;

#[cfg(test)]
#[path = "../tests/capi.rs"]
mod capi_tests;

#[cfg(test)]
#[path = "../tests/headers.rs"]
mod header_tests;
