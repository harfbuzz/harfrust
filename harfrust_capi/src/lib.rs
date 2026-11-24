/*!
A C API for [HarfRust](https://github.com/harfbuzz/harfrust), mirroring the
shaping half of HarfBuzz's API with an `hr_` prefix in place of `hb_`.

Every type, function, enumerator and constant is named and numbered to match
its HarfBuzz counterpart, so C code can usually be ported by renaming `hb_` to
`hr_`. The prefix also means this library can be linked into the same process
as HarfBuzz itself without collisions.

# Scope

This covers shaping: blobs, faces, fonts, buffers and `hr_shape`, plus OpenType
`BASE` baseline and `MATH` queries. It also provides layout script/feature and lookup queries,
OpenType script and language tag conversion, and the complete set and map
container APIs. The optional `subset` feature adds Skera-backed font subsetting.
Drawing and painting callbacks are outside this API.

# Object lifetime

Objects are reference counted. Constructors return a new reference the caller
owns; `hr_*_reference` takes another, and `hr_*_destroy` gives one back. The
`hr_*_create` functions never return `NULL`: on failure they return an
immortal empty object, and the `_or_fail` variants return `NULL` instead. Every
getter also accepts `NULL`, behaving as though it were passed the empty object.

A reference taken to an object from one of its own destroy callbacks, while
its last reference is being released, does not keep it alive. It must be given
back, along with anything that holds it, before that callback returns.
*/

// The whole point of this crate is to look like C.
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

pub mod aat_layout;
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

/// cbindgen:ignore
#[cfg(feature = "draw")]
pub mod draw;
/// cbindgen:ignore
#[cfg(feature = "paint")]
pub mod paint;
#[cfg(feature = "draw")]
mod rendering;

/// cbindgen:ignore
#[cfg(feature = "subset")]
pub mod subset;

pub use aat_layout::*;
pub use blob::*;
pub use buffer::*;
pub use color::*;
pub use common::*;
#[cfg(feature = "draw")]
pub use draw::*;
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
#[cfg(feature = "paint")]
pub use paint::*;
pub use set::*;
pub use shape::*;
pub use shape_plan::*;
#[cfg(feature = "subset")]
pub use subset::*;
pub use unicode::*;

// The documentation says which objects may be used from several threads at
// once. Every use of one goes through a raw pointer, so the compiler is never
// asked whether that holds. This asks it, about the Rust types those objects
// keep their contents in. It says nothing about what an object holds through
// a raw pointer, nor about a type that is not listed here.
const _: () = {
    const fn shared_between_threads<T: Send + Sync>() {}

    // A face and its fonts are shaped with from several threads at once.
    shared_between_threads::<harfrust::font::Font>();
    // The plans in a face's cache are read by every thread shaping through it.
    shared_between_threads::<harfrust::ShapePlan>();
    // That cache is added to by whichever of those threads misses.
    shared_between_threads::<plan::PlanCache>();
    // A set that nothing is modifying is read from several threads at once.
    shared_between_threads::<SetValues>();
    // So is a map, which keeps its entries in fields of its own.
    shared_between_threads::<hr_map_t>();
    // A face's outlines are built once, by whichever thread draws first.
    #[cfg(feature = "draw")]
    shared_between_threads::<rendering::RenderData>();
    // A subset plan may be executed from several threads at once.
    #[cfg(feature = "subset")]
    shared_between_threads::<skera::Plan>();
};

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

#[cfg(all(test, feature = "draw"))]
#[path = "../tests/rendering.rs"]
mod rendering_tests;

#[cfg(test)]
#[path = "../tests/tables.rs"]
mod table_tests;

#[cfg(test)]
#[path = "../tests/ot_queries.rs"]
mod ot_query_tests;

#[cfg(test)]
#[path = "../tests/ot_color.rs"]
mod ot_color_tests;

#[cfg(test)]
#[path = "../tests/axis_names.rs"]
mod axis_name_tests;

#[cfg(test)]
#[path = "../tests/aat_metadata.rs"]
mod aat_metadata_tests;
