/*!
A C API for [HarfRust](https://github.com/harfbuzz/harfrust), mirroring the
shaping half of HarfBuzz's API with an `hr_` prefix in place of `hb_`.

Every type, function, enumerator and constant is named and numbered to match
its HarfBuzz counterpart, so C code can usually be ported by renaming `hb_` to
`hr_`. The prefix also means this library can be linked into the same process
as HarfBuzz itself without collisions.

# Scope

This covers shaping: blobs, faces, fonts, buffers and `hr_shape`, plus OpenType
`BASE` baseline and `MATH` queries. It also provides layout lookup queries,
OpenType script and language tag conversion, and the complete set and map
container APIs. Drawing and painting callbacks and subsetting are outside this API.

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
pub mod color;
pub mod common;
pub mod face;
pub mod font;
pub mod font_funcs;
pub mod map;
pub mod object;
pub mod ot_color;
pub mod ot_layout;
pub mod ot_math;
pub mod ot_name;
pub mod ot_tag;
pub mod ot_var;
mod plan;
pub mod set;
pub mod shape;
pub mod shape_plan;
pub mod unicode;

pub use blob::*;
pub use buffer::*;
pub use color::*;
pub use common::*;
pub use face::*;
pub use font::*;
pub use font_funcs::*;
pub use map::*;
pub use object::{hr_destroy_func_t, hr_user_data_key_t};
pub use ot_color::*;
pub use ot_layout::*;
pub use ot_math::*;
pub use ot_name::*;
pub use ot_tag::*;
pub use ot_var::*;
pub use set::*;
pub use shape::*;
pub use shape_plan::*;
pub use unicode::*;

#[cfg(test)]
#[path = "../tests/capi.rs"]
mod capi_tests;

#[cfg(test)]
#[path = "../tests/containers.rs"]
mod container_tests;

#[cfg(test)]
#[path = "../tests/headers.rs"]
mod header_tests;

#[cfg(test)]
#[path = "../tests/skia_face.rs"]
mod skia_face_tests;

#[cfg(test)]
#[path = "../tests/skia_batch.rs"]
mod skia_batch_tests;

#[cfg(test)]
#[path = "../tests/skia_version.rs"]
mod skia_version_tests;

#[cfg(test)]
#[path = "../tests/tables.rs"]
mod table_tests;

#[cfg(test)]
#[path = "../tests/ot_color.rs"]
mod ot_color_tests;

#[cfg(test)]
#[path = "../tests/axis_names.rs"]
mod axis_name_tests;
