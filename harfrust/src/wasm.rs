//! Host for fonts that carry their own WebAssembly shaping engine.
//!
//! A font with a `Wasm` table is shaped by the module in that table, using
//! the same host interface as HarfBuzz's optional WebAssembly shaper. See
//! `harfbuzz/docs/wasm-shaper.md` for the protocol.
//!
//! The implementation uses [`wasmi`], a pure Rust interpreter that builds
//! for `no_std` targets, so a HarfRust build with an embedded WebAssembly
//! shaper can itself be compiled to `wasm32-unknown-unknown`.
//!
//! Explicit lifetimes on the host import functions are needed because they
//! are passed as function items; clippy cannot see that through `func_wrap`.
#![allow(clippy::needless_lifetimes, clippy::elidable_lifetime_names)]

#[cfg(feature = "std")]
use alloc::string::String;
use alloc::vec::Vec;
#[cfg(feature = "std")]
use core::fmt::Write as _;
#[cfg(not(feature = "std"))]
#[allow(unused_imports)]
use core_maths::CoreFloat as _;

use read_fonts::model::glyph::outline::{DrawMemory, DrawOptions, PathSink};
use read_fonts::model::{Font as ModelFont, Kind, Source};
use read_fonts::types::{F2Dot14, GlyphId, Tag as RawTag};
use read_fonts::{FontRef, TableProvider};
use wasmi::{Caller, Engine, Extern, ExternType, Linker, Memory, Module, Store, Val};

use crate::{
    Buffer, Direction, Feature, GlyphInfo, GlyphPosition, Script, ShapeError, ShapeOptions, Shaper,
    ShaperFont, Tag,
};

const WASM_TAG: Tag = Tag::new(b"Wasm");
const BUFFER_TOKEN: u32 = 1;
const ROOT_FACE_TOKEN: u32 = 1;
const ROOT_FONT_TOKEN: u32 = 1;

const DIRECTION_INVALID: u32 = 0;
const DIRECTION_LTR: u32 = 4;
const DIRECTION_RTL: u32 = 5;
const DIRECTION_TTB: u32 = 6;
const DIRECTION_BTT: u32 = 7;

const GLYPH_INFO_SIZE: u32 = 20;
const GLYPH_POSITION_SIZE: u32 = 20;
const FEATURE_SIZE: u32 = 16;
const OUTLINE_POINT_SIZE: u32 = 12;

const MOVE_TO: u32 = 0;
const LINE_TO: u32 = 1;
const QUADRATIC_TO: u32 = 2;
const CUBIC_TO: u32 = 3;

// What a font's module may use for one `shape` call. The policy is the one in
// HarfBuzz's WebAssembly shaper (`hb-limits.hh`), in this engine's units. A
// call that goes over fails with `ShapeError::WasmShapingFailed`.

/// Fuel for a call: this much, plus [`FUEL_PER_ITEM`] for each item in the
/// buffer. It covers making the instance, `shape_plan_create` and `shape`,
/// which all happen on every call here.
const FUEL_MIN: u64 = 1 << 27;
/// wasmi charges 1.1 to 1.4 times as much fuel as wasm-micro-runtime counts
/// instructions for the same module, so this is HarfBuzz's 2^22 times one
/// and a half.
const FUEL_PER_ITEM: u64 = 3 << 21;

/// Memory for a call, in bytes: this much, plus [`MEMORY_FONT_FACTOR`] times
/// the size of the font, plus [`MEMORY_BUFFER_FACTOR`] times the size of the
/// buffer's contents. The data of the faces and fonts a module creates, which
/// the host holds, counts against the same allowance.
const MEMORY_MIN: u64 = 32 << 20;
const MEMORY_FONT_FACTOR: u64 = 2;
const MEMORY_BUFFER_FACTOR: u64 = 128;

/// Faces and fonts a module may create in a call.
const MAX_OBJECTS: usize = 256;

/// What calling a host function costs a module in fuel, before any work
/// that grows with a size. See [`charge`].
const CALL_COST: u64 = 64;

/// A compiled module, or a marker saying the font's module cannot run.
pub(crate) enum WasmModuleSlot {
    Compiled { engine: Engine, module: Module },
    Unavailable,
}

/// Returns true if the font carries a `Wasm` table that can be read.
pub(crate) fn has_wasm_table(font: &ModelFont) -> bool {
    wasm_table_data(font).is_some()
}

fn wasm_table_data<'a>(font: &'a ModelFont) -> Option<&'a [u8]> {
    let (Source::Blob(blob), Kind::Sfnt(_, index)) = (font.source(), font.kind()) else {
        return None;
    };
    let font_ref = FontRef::from_index(blob, index).ok()?;
    font_ref.table_data(WASM_TAG).map(|data| data.as_bytes())
}

fn compile_slot(font: &ModelFont) -> WasmModuleSlot {
    let Some(bytes) = wasm_table_data(font) else {
        return WasmModuleSlot::Unavailable;
    };
    // Fuel is what stops a module that does not stop by itself.
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    match Module::new(&engine, bytes) {
        Ok(module) => WasmModuleSlot::Compiled { engine, module },
        Err(_) => WasmModuleSlot::Unavailable,
    }
}

/// Shapes with the font's WebAssembly module.
///
/// Returns `None` when the font has no usable `Wasm` table, so the caller
/// can fall back to the OpenType shaper.
pub(crate) fn shape<'a, 'b, 'c>(
    font: &'a ShaperFont<'b, 'c>,
    buffer: &'a mut Buffer,
    options: &ShapeOptions<'_>,
) -> Option<Result<(), ShapeError>> {
    let model: &ModelFont = font;
    if !has_wasm_table(model) {
        return None;
    }
    let cache = crate::shaper_font::layout_cache(model)?;
    let slot = cache.wasm.get_or_init(|| compile_slot(model));
    match slot {
        WasmModuleSlot::Unavailable => None,
        WasmModuleSlot::Compiled { engine, module } => {
            buffer.enter();
            let result = run(engine, module, font, buffer, options);
            buffer.leave();
            match result {
                Ok(Some(())) => Some(Ok(())),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
        }
    }
}

/// A face created by the guest from a blob.
struct WasmFace {
    data: Vec<u8>,
    index: u32,
}

/// A font created by the guest from a face.
struct WasmFontEntry {
    face: u32,
    font: ModelFont,
    x_scale: i32,
    y_scale: i32,
}

/// The host state visible to guest imports.
struct Host<'a, 'b, 'c> {
    font: &'a ShaperFont<'b, 'c>,
    buffer: &'a mut Buffer,
    point_size: Option<f32>,
    /// Faces and fonts the guest created. A token is the index plus 2, and
    /// is not used again once the guest has destroyed what it named.
    faces: Vec<Option<WasmFace>>,
    fonts: Vec<Option<WasmFontEntry>>,
    memory: Option<Memory>,
    /// Next address for the host-side bump allocator. Allocations live in
    /// pages the host adds to the end of the memory for itself, so they
    /// cannot collide with the guest's own heap.
    bump_next: u32,
    /// End of the pages the allocator has taken.
    bump_end: u32,
    /// The memory the module may use, in bytes.
    max_memory: u64,
    /// How much of it is in faces and fonts that the module created.
    object_bytes: u64,
    limits: wasmi::StoreLimits,
}

impl<'a, 'b, 'c> Host<'a, 'b, 'c> {
    fn new(
        font: &'a ShaperFont<'b, 'c>,
        buffer: &'a mut Buffer,
        point_size: Option<f32>,
        max_memory: u64,
    ) -> Self {
        Self {
            font,
            buffer,
            point_size,
            faces: Vec::new(),
            fonts: Vec::new(),
            memory: None,
            bump_next: 0,
            bump_end: 0,
            max_memory,
            object_bytes: 0,
            limits: wasmi::StoreLimitsBuilder::new()
                .memory_size(usize::try_from(max_memory).unwrap_or(usize::MAX))
                .table_elements(1 << 16)
                .instances(1)
                .memories(1)
                .tables(4)
                .build(),
        }
    }

    /// Whether the module may create one more face or font, with this much
    /// data for the host to hold.
    fn can_create(&self, bytes: u64) -> bool {
        self.faces.len() + self.fonts.len() < MAX_OBJECTS
            && bytes <= self.max_memory.saturating_sub(self.object_bytes)
    }
}

fn run<'a, 'b, 'c>(
    engine: &Engine,
    module: &Module,
    font: &'a ShaperFont<'b, 'c>,
    buffer: &'a mut Buffer,
    options: &ShapeOptions<'_>,
) -> Result<Option<()>, ShapeError> {
    let items = buffer.len as u64;
    let max_memory = MEMORY_MIN
        .saturating_add(MEMORY_FONT_FACTOR.saturating_mul(font_data_len(font)))
        .saturating_add(
            MEMORY_BUFFER_FACTOR
                .saturating_mul(u64::from(GLYPH_INFO_SIZE + GLYPH_POSITION_SIZE))
                .saturating_mul(items),
        );
    let fuel = FUEL_MIN.saturating_add(FUEL_PER_ITEM.saturating_mul(items));

    let host = Host::new(font, buffer, options.point_size, max_memory);
    let mut store = Store::new(engine, host);
    store.limiter(|host| &mut host.limits);
    // The start function, if there is one, runs on this fuel too.
    if store.set_fuel(fuel).is_err() {
        return Ok(None);
    }
    let mut linker: Linker<Host<'a, 'b, 'c>> = Linker::new(engine);
    if define_links(&mut linker, module).is_err() {
        // The module imports something we cannot provide; use OpenType.
        return Ok(None);
    }
    let Ok(instance_pre) = linker.instantiate(&mut store, module) else {
        return Ok(None);
    };
    let Ok(instance) = instance_pre.start(&mut store) else {
        return Ok(None);
    };

    let memory = instance
        .get_export(&store, "memory")
        .and_then(Extern::into_memory);
    let Some(memory) = memory else {
        // A shaper without linear memory cannot exchange buffer contents.
        return Ok(None);
    };
    store.data_mut().memory = Some(memory);

    let Ok(shape_func) = instance.get_typed_func::<(i32, i32, i32, i32, i32), i32>(&store, "shape")
    else {
        // No shape entry point; leave the font to the OpenType shaper.
        return Ok(None);
    };

    let plan_token = instance
        .get_typed_func::<i32, i32>(&store, "shape_plan_create")
        .ok()
        .and_then(|func| func.call(&mut store, ROOT_FACE_TOKEN as i32).ok())
        .unwrap_or(0);

    let mut features_ptr = 0;
    if !options.features.is_empty() {
        let size = options.features.len() as u32 * FEATURE_SIZE;
        let Some(ptr) = store_alloc(&mut store, memory, size, 4) else {
            return Err(ShapeError::WasmShapingFailed);
        };
        features_ptr = i32::try_from(ptr).unwrap_or(0);
        for (index, feature) in options.features.iter().enumerate() {
            let base = ptr + index as u32 * FEATURE_SIZE;
            let tag = u32::from_be_bytes(feature.tag.to_be_bytes());
            let Some(()) = write_u32(&mut store, memory, base, tag) else {
                return Err(ShapeError::WasmShapingFailed);
            };
            let Some(()) = write_u32(&mut store, memory, base + 4, feature.value) else {
                return Err(ShapeError::WasmShapingFailed);
            };
            let Some(()) = write_u32(&mut store, memory, base + 8, feature.start) else {
                return Err(ShapeError::WasmShapingFailed);
            };
            let Some(()) = write_u32(&mut store, memory, base + 12, feature.end) else {
                return Err(ShapeError::WasmShapingFailed);
            };
        }
    }

    let status = shape_func.call(
        &mut store,
        (
            plan_token,
            ROOT_FONT_TOKEN as i32,
            BUFFER_TOKEN as i32,
            features_ptr,
            i32::try_from(options.features.len()).unwrap_or(0),
        ),
    );

    let Ok(status) = status else {
        return Err(ShapeError::WasmShapingFailed);
    };
    if status == 0 {
        return Err(ShapeError::WasmShapingFailed);
    }

    let buffer = &mut *store.data_mut().buffer;
    buffer.content_type = Some(crate::ContentType::Glyphs);
    for info in buffer.info.iter_mut().take(buffer.len) {
        info.mask &= !crate::GlyphFlags::DEFINED_BITS;
    }
    buffer.unsafe_to_break(None, None);
    Ok(Some(()))
}

type KnownImport = (&'static str, &'static str);

const KNOWN_IMPORTS: &[KnownImport] = &[
    ("env", "script_get_horizontal_direction"),
    ("env", "blob_free"),
    ("env", "buffer_contents_realloc"),
    ("env", "buffer_contents_free"),
    ("env", "buffer_copy_contents"),
    ("env", "buffer_set_contents"),
    ("env", "buffer_get_direction"),
    ("env", "buffer_get_script"),
    ("env", "buffer_reverse"),
    ("env", "buffer_reverse_clusters"),
    ("env", "face_create"),
    ("env", "face_copy_table"),
    ("env", "face_get_upem"),
    ("env", "face_destroy"),
    ("env", "font_create"),
    ("env", "font_destroy"),
    ("env", "font_get_face"),
    ("env", "font_get_scale"),
    ("env", "font_get_glyph"),
    ("env", "font_get_glyph_h_advance"),
    ("env", "font_get_glyph_v_advance"),
    ("env", "font_get_glyph_extents"),
    ("env", "font_glyph_to_string"),
    ("env", "font_copy_glyph_outline"),
    ("env", "coords_free"),
    ("env", "font_copy_coords"),
    ("env", "font_set_coords"),
    ("env", "glyph_outline_free"),
    ("env", "shape_with"),
    ("env", "feature_copy_contents"),
    ("env", "features_free"),
    ("env", "debugprint"),
    ("env", "debugprint1"),
    ("env", "debugprint2"),
    ("env", "debugprint3"),
    ("env", "debugprint4"),
];

fn define_links<'a, 'b, 'c>(
    linker: &mut Linker<Host<'a, 'b, 'c>>,
    module: &Module,
) -> Result<(), wasmi::Error> {
    linker.func_wrap(
        "env",
        "script_get_horizontal_direction",
        api_script_get_horizontal_direction,
    )?;
    linker.func_wrap("env", "blob_free", api_blob_free)?;
    linker.func_wrap(
        "env",
        "buffer_contents_realloc",
        api_buffer_contents_realloc,
    )?;
    linker.func_wrap("env", "buffer_contents_free", api_buffer_contents_free)?;
    linker.func_wrap("env", "buffer_copy_contents", api_buffer_copy_contents)?;
    linker.func_wrap("env", "buffer_set_contents", api_buffer_set_contents)?;
    linker.func_wrap("env", "buffer_get_direction", api_buffer_get_direction)?;
    linker.func_wrap("env", "buffer_get_script", api_buffer_get_script)?;
    linker.func_wrap("env", "buffer_reverse", api_buffer_reverse)?;
    linker.func_wrap(
        "env",
        "buffer_reverse_clusters",
        api_buffer_reverse_clusters,
    )?;
    linker.func_wrap("env", "face_create", api_face_create)?;
    linker.func_wrap("env", "face_copy_table", api_face_copy_table)?;
    linker.func_wrap("env", "face_get_upem", api_face_get_upem)?;
    linker.func_wrap("env", "face_destroy", api_face_destroy)?;
    linker.func_wrap("env", "font_create", api_font_create)?;
    linker.func_wrap("env", "font_destroy", api_font_destroy)?;
    linker.func_wrap("env", "font_get_face", api_font_get_face)?;
    linker.func_wrap("env", "font_get_scale", api_font_get_scale)?;
    linker.func_wrap("env", "font_get_glyph", api_font_get_glyph)?;
    linker.func_wrap(
        "env",
        "font_get_glyph_h_advance",
        api_font_get_glyph_h_advance,
    )?;
    linker.func_wrap(
        "env",
        "font_get_glyph_v_advance",
        api_font_get_glyph_v_advance,
    )?;
    linker.func_wrap("env", "font_get_glyph_extents", api_font_get_glyph_extents)?;
    linker.func_wrap("env", "font_glyph_to_string", api_font_glyph_to_string)?;
    linker.func_wrap(
        "env",
        "font_copy_glyph_outline",
        api_font_copy_glyph_outline,
    )?;
    linker.func_wrap("env", "coords_free", api_coords_free)?;
    linker.func_wrap("env", "font_copy_coords", api_font_copy_coords)?;
    linker.func_wrap("env", "font_set_coords", api_font_set_coords)?;
    linker.func_wrap("env", "glyph_outline_free", api_glyph_outline_free)?;
    linker.func_wrap("env", "shape_with", api_shape_with)?;
    linker.func_wrap("env", "feature_copy_contents", api_feature_copy_contents)?;
    linker.func_wrap("env", "features_free", api_features_free)?;
    linker.func_wrap("env", "debugprint", api_debugprint)?;
    linker.func_wrap("env", "debugprint1", api_debugprint1)?;
    linker.func_wrap("env", "debugprint2", api_debugprint2)?;
    linker.func_wrap("env", "debugprint3", api_debugprint3)?;
    linker.func_wrap("env", "debugprint4", api_debugprint4)?;

    // wasm-bindgen and friends may import helpers we do not implement. Define
    // zero-returning stubs so a module that never calls them still loads.
    for import in module.imports() {
        let module_name = import.module();
        let name = import.name();
        if KNOWN_IMPORTS
            .iter()
            .any(|(known_module, known_name)| *known_module == module_name && *known_name == name)
        {
            continue;
        }
        let ExternType::Func(func_type) = import.ty().clone() else {
            continue;
        };
        let results = func_type.results().to_vec();
        linker.func_new(
            module_name,
            name,
            func_type,
            move |_caller, _params, outputs| {
                for (index, ty) in results.iter().enumerate() {
                    outputs[index] = Val::default(*ty);
                }
                Ok(())
            },
        )?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Guest memory helpers
// ---------------------------------------------------------------------------

fn read_u32<T>(ctx: impl wasmi::AsContext<Data = T>, memory: Memory, ptr: u32) -> Option<u32> {
    let mut bytes = [0u8; 4];
    memory
        .read(ctx, usize::try_from(ptr).ok()?, &mut bytes)
        .ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_i32<T>(ctx: impl wasmi::AsContext<Data = T>, memory: Memory, ptr: u32) -> Option<i32> {
    Some(read_u32(ctx, memory, ptr)? as i32)
}

fn write_u32<T>(
    ctx: impl wasmi::AsContextMut<Data = T>,
    memory: Memory,
    ptr: u32,
    value: u32,
) -> Option<()> {
    memory
        .write(ctx, usize::try_from(ptr).ok()?, &value.to_le_bytes())
        .ok()
}

fn write_i32<T>(
    ctx: impl wasmi::AsContextMut<Data = T>,
    memory: Memory,
    ptr: u32,
    value: i32,
) -> Option<()> {
    write_u32(ctx, memory, ptr, value as u32)
}

fn write_f32<T>(
    ctx: impl wasmi::AsContextMut<Data = T>,
    memory: Memory,
    ptr: u32,
    value: f32,
) -> Option<()> {
    write_u32(ctx, memory, ptr, value.to_bits())
}

fn read_bytes<T>(
    ctx: impl wasmi::AsContext<Data = T>,
    memory: Memory,
    ptr: u32,
    len: u32,
) -> Option<Vec<u8>> {
    let start = usize::try_from(ptr).ok()?;
    let len = usize::try_from(len).ok()?;
    if start.checked_add(len)? > memory.data_size(&ctx) {
        return None;
    }
    let mut bytes = alloc::vec![0u8; len];
    memory.read(ctx, start, &mut bytes).ok()?;
    Some(bytes)
}

fn write_bytes<T>(
    ctx: impl wasmi::AsContextMut<Data = T>,
    memory: Memory,
    ptr: u32,
    bytes: &[u8],
) -> Option<()> {
    memory.write(ctx, usize::try_from(ptr).ok()?, bytes).ok()
}

fn read_cstr<T>(ctx: impl wasmi::AsContext<Data = T>, memory: Memory, ptr: u32) -> Option<Vec<u8>> {
    let size = memory.data_size(&ctx);
    let start = usize::try_from(ptr).ok()?;
    if start >= size {
        return None;
    }
    let mut output = Vec::new();
    let mut pos = start;
    let mut chunk = [0u8; 256];
    while pos < size && output.len() < 4096 {
        let len = (size - pos).min(chunk.len());
        memory.read(&ctx, pos, &mut chunk[..len]).ok()?;
        if let Some(end) = chunk[..len].iter().position(|&byte| byte == 0) {
            output.extend_from_slice(&chunk[..end]);
            return Some(output);
        }
        output.extend_from_slice(&chunk[..len]);
        pos += len;
    }
    Some(output)
}

fn align_up(value: u32, align: u32) -> Option<u32> {
    let align = align.max(8);
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}

/// Safely add an offset to a pointer, returning None on overflow.
fn ptr_add(ptr: u32, offset: u32) -> Option<u32> {
    ptr.checked_add(offset)
}

/// The size of the font's data.
fn font_data_len(font: &ModelFont) -> u64 {
    match font.source() {
        Source::Blob(blob) => blob.as_ref().len() as u64,
        Source::TableFunction(_) => 0,
    }
}

/// Takes `cost` fuel from the module. Returns false if it has less: the
/// module is then left with none, and stops at its next block.
fn spend<T>(caller: &mut Caller<'_, T>, cost: u64) -> bool {
    match caller.get_fuel() {
        Ok(fuel) if fuel >= cost => caller.set_fuel(fuel - cost).is_ok(),
        _ => {
            let _ = caller.set_fuel(0);
            false
        }
    }
}

/// Charges the module for a call into the host: [`CALL_COST`], and `cost` for
/// work that grows with a size.
///
/// The fuel wasmi counts is for the module's own instructions. Without this a
/// module could keep the host busy, copying a table again and again, for as
/// long as it liked on very little fuel. The amounts need not be exact, only
/// not so low that host work is the cheap way to spend time.
fn charge<T>(caller: &mut Caller<'_, T>, cost: u64) -> bool {
    spend(caller, CALL_COST.saturating_add(cost))
}

/// What copying this many bytes costs.
fn bytes_cost(bytes: u64) -> u64 {
    bytes / 8
}

/// Allocates from the guest memory for the host.
///
/// The host adds pages to the end of the memory and hands them out in order.
/// Nothing is freed; the memory goes with the instance at the end of the
/// call.
fn bump_alloc<'a, 'b: 'a, 'c: 'a>(
    mut ctx: impl wasmi::AsContextMut<Data = Host<'a, 'b, 'c>>,
    memory: Memory,
    size: u32,
    align: u32,
) -> Option<u32> {
    let size = size.max(1);

    // What is left of the pages taken before.
    let (next, end) = {
        let ctx = ctx.as_context();
        let host = ctx.data();
        (host.bump_next, host.bump_end)
    };
    if let Some(start) = align_up(next, align) {
        if let Some(stop) = start.checked_add(size) {
            if stop <= end {
                ctx.as_context_mut().data_mut().bump_next = stop;
                return Some(start);
            }
        }
    }

    // New pages. The memory's size is a whole number of them, which is
    // aligned enough for anything.
    let start = u32::try_from(memory.data_size(&ctx)).ok()?;
    let pages = size.div_ceil(65536);
    let stop = start.checked_add(size)?;
    let end = start.checked_add(pages.checked_mul(65536)?)?;
    memory.grow(&mut ctx, u64::from(pages)).ok()?;
    let mut ctx = ctx.as_context_mut();
    let host = ctx.data_mut();
    host.bump_next = stop;
    host.bump_end = end;
    Some(start)
}

/// Allocates from the guest memory used by a host function.
fn host_alloc(caller: &mut Caller<'_, Host<'_, '_, '_>>, size: u32, align: u32) -> Option<u32> {
    let memory = caller.data().memory?;
    bump_alloc(caller, memory, size, align)
}

/// Allocates from the guest memory outside a host function.
fn store_alloc(
    store: &mut Store<Host<'_, '_, '_>>,
    memory: Memory,
    size: u32,
    align: u32,
) -> Option<u32> {
    bump_alloc(store, memory, size, align)
}

fn bytes_for(count: u32, item_size: u32) -> Option<u32> {
    count.checked_mul(item_size)
}

// ---------------------------------------------------------------------------
// Buffer exchange
// ---------------------------------------------------------------------------

fn api_buffer_copy_contents<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    buffer_token: i32,
    contents_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    if buffer_token as u32 != BUFFER_TOKEN {
        return 0;
    }
    let contents_ptr = contents_ptr as u32;
    let Some(capacity) = read_u32(&caller, memory, contents_ptr) else {
        return 0;
    };

    let state = caller.data_mut();
    if state.buffer.have_output {
        state.buffer.sync();
    }
    if !state.buffer.have_positions {
        state.buffer.clear_positions();
    }
    let length = u32::try_from(state.buffer.len).unwrap_or(u32::MAX);
    if !charge(
        &mut caller,
        bytes_cost(u64::from(length) * u64::from(GLYPH_INFO_SIZE + GLYPH_POSITION_SIZE)),
    ) {
        return 0;
    }

    let (info_ptr, pos_ptr) = if length <= capacity {
        let Some(info_ptr) = read_u32(&caller, memory, contents_ptr + 4) else {
            return 0;
        };
        let Some(pos_ptr) = read_u32(&caller, memory, contents_ptr + 8) else {
            return 0;
        };
        if length > 0 && (info_ptr == 0 || pos_ptr == 0) {
            return 0;
        }
        (info_ptr, pos_ptr)
    } else {
        let Some(info_bytes) = bytes_for(length, GLYPH_INFO_SIZE) else {
            return 0;
        };
        let Some(new_info) = host_alloc(&mut caller, info_bytes, 4) else {
            return 0;
        };
        let Some(new_pos) = host_alloc(&mut caller, info_bytes, 4) else {
            return 0;
        };
        let _ = write_u32(&mut caller, memory, contents_ptr, length);
        let _ = write_u32(&mut caller, memory, contents_ptr + 4, new_info);
        let _ = write_u32(&mut caller, memory, contents_ptr + 8, new_pos);
        (new_info, new_pos)
    };

    for index in 0..usize::try_from(length).unwrap_or(0) {
        let (info_fields, position_fields) = {
            let state = caller.data();
            let buffer = &*state.buffer;
            let Some(info) = buffer.info.get(index) else {
                return 0;
            };
            let Some(position) = buffer.pos.get(index) else {
                return 0;
            };
            (
                [
                    info.glyph_id,
                    info.mask,
                    info.cluster,
                    info.vars[0],
                    info.vars[1],
                ],
                [
                    position.x_advance as u32,
                    position.y_advance as u32,
                    position.x_offset as u32,
                    position.y_offset as u32,
                    position.var,
                ],
            )
        };
        let base = info_ptr + index as u32 * GLYPH_INFO_SIZE;
        for (field, value) in info_fields.iter().enumerate() {
            if write_u32(&mut caller, memory, base + field as u32 * 4, *value).is_none() {
                return 0;
            }
        }
        let base = pos_ptr + index as u32 * GLYPH_POSITION_SIZE;
        for (field, value) in position_fields.iter().enumerate() {
            if write_u32(&mut caller, memory, base + field as u32 * 4, *value).is_none() {
                return 0;
            }
        }
    }
    1
}

fn api_buffer_set_contents<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    buffer_token: i32,
    contents_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    if buffer_token as u32 != BUFFER_TOKEN {
        return 0;
    }
    let contents_ptr = contents_ptr as u32;
    let Some(length) = read_u32(&caller, memory, contents_ptr) else {
        return 0;
    };
    let Some(info_ptr) = read_u32(&caller, memory, contents_ptr + 4) else {
        return 0;
    };
    let Some(pos_ptr) = read_u32(&caller, memory, contents_ptr + 8) else {
        return 0;
    };
    if length > 0 && (info_ptr == 0 || pos_ptr == 0) {
        return 0;
    }
    if !charge(
        &mut caller,
        bytes_cost(u64::from(length) * u64::from(GLYPH_INFO_SIZE + GLYPH_POSITION_SIZE)),
    ) {
        return 0;
    }
    let Some(infos) = read_infos(&caller, memory, info_ptr, length) else {
        return 0;
    };
    let Some(positions) = read_positions(&caller, memory, pos_ptr, length) else {
        return 0;
    };

    let state = caller.data_mut();
    if !state
        .buffer
        .set_length(usize::try_from(length).unwrap_or(usize::MAX))
    {
        return 0;
    }
    if !state.buffer.have_positions {
        state.buffer.clear_positions();
    }
    let buffer = &mut *state.buffer;
    if let Some(slot) = buffer.info.get_mut(..infos.len()) {
        slot.copy_from_slice(&infos);
    } else {
        return 0;
    }
    if !positions.is_empty() {
        if let Some(slot) = buffer.pos.get_mut(..positions.len()) {
            slot.copy_from_slice(&positions);
        } else {
            return 0;
        }
    }
    1
}

fn read_infos<T>(
    ctx: impl wasmi::AsContext<Data = T>,
    memory: Memory,
    ptr: u32,
    length: u32,
) -> Option<Vec<GlyphInfo>> {
    let bytes = bytes_for(length, GLYPH_INFO_SIZE)?;
    if usize::try_from(ptr)
        .ok()?
        .checked_add(usize::try_from(bytes).ok()?)?
        > memory.data_size(&ctx)
    {
        return None;
    }
    let mut result = Vec::with_capacity(usize::try_from(length).ok()?);
    for index in 0..length {
        let base = ptr + index.checked_mul(GLYPH_INFO_SIZE)?;
        result.push(GlyphInfo {
            glyph_id: read_u32(&ctx, memory, base)?,
            mask: read_u32(&ctx, memory, base + 4)?,
            cluster: read_u32(&ctx, memory, base + 8)?,
            vars: [
                read_u32(&ctx, memory, base + 12)?,
                read_u32(&ctx, memory, base + 16)?,
            ],
        });
    }
    Some(result)
}

fn read_positions<T>(
    ctx: impl wasmi::AsContext<Data = T>,
    memory: Memory,
    ptr: u32,
    length: u32,
) -> Option<Vec<GlyphPosition>> {
    let bytes = bytes_for(length, GLYPH_POSITION_SIZE)?;
    if usize::try_from(ptr)
        .ok()?
        .checked_add(usize::try_from(bytes).ok()?)?
        > memory.data_size(&ctx)
    {
        return None;
    }
    let mut result = Vec::with_capacity(usize::try_from(length).ok()?);
    for index in 0..length {
        let base = ptr + index.checked_mul(GLYPH_POSITION_SIZE)?;
        result.push(GlyphPosition {
            x_advance: read_i32(&ctx, memory, base)?,
            y_advance: read_i32(&ctx, memory, base + 4)?,
            x_offset: read_i32(&ctx, memory, base + 8)?,
            y_offset: read_i32(&ctx, memory, base + 12)?,
            var: read_u32(&ctx, memory, base + 16)?,
        });
    }
    Some(result)
}

fn api_buffer_contents_realloc<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    contents_ptr: i32,
    size: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let contents_ptr = contents_ptr as u32;
    let size = size as u32;
    let Some(length) = read_u32(&caller, memory, contents_ptr) else {
        return 0;
    };
    if size <= length {
        return 1;
    }
    let Some(old_info) = read_u32(&caller, memory, contents_ptr + 4) else {
        return 0;
    };
    let Some(old_pos) = read_u32(&caller, memory, contents_ptr + 8) else {
        return 0;
    };
    let Some(bytes) = bytes_for(size, GLYPH_INFO_SIZE) else {
        return 0;
    };
    if !charge(&mut caller, bytes_cost(u64::from(bytes) * 2)) {
        return 0;
    }
    let Some(new_info) = host_alloc(&mut caller, bytes, 4) else {
        return 0;
    };
    let Some(new_pos) = host_alloc(&mut caller, bytes, 4) else {
        return 0;
    };
    if length > 0 {
        let Some(old_bytes) = bytes_for(length, GLYPH_INFO_SIZE) else {
            return 0;
        };
        let Some(infos) = read_bytes(&caller, memory, old_info, old_bytes) else {
            return 0;
        };
        if write_bytes(&mut caller, memory, new_info, &infos).is_none() {
            return 0;
        }
        let Some(positions) = read_bytes(&caller, memory, old_pos, old_bytes) else {
            return 0;
        };
        if write_bytes(&mut caller, memory, new_pos, &positions).is_none() {
            return 0;
        }
    }
    let _ = write_u32(&mut caller, memory, contents_ptr, size);
    let _ = write_u32(&mut caller, memory, contents_ptr + 4, new_info);
    let _ = write_u32(&mut caller, memory, contents_ptr + 8, new_pos);
    1
}

fn api_buffer_contents_free<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    contents_ptr: i32,
) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 0) {
        return;
    }
    let contents_ptr = contents_ptr as u32;
    for offset in [0, 4, 8] {
        if let Some(addr) = ptr_add(contents_ptr, offset) {
            let _ = write_u32(&mut caller, memory, addr, 0);
        }
    }
}

fn api_buffer_get_direction<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    buffer_token: i32,
) -> i32 {
    if buffer_token as u32 != BUFFER_TOKEN || !charge(&mut caller, 0) {
        return DIRECTION_INVALID as i32;
    }
    direction_to_wasm(caller.data().buffer.direction) as i32
}

fn api_buffer_get_script<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    buffer_token: i32,
) -> i32 {
    if buffer_token as u32 != BUFFER_TOKEN || !charge(&mut caller, 0) {
        return 0;
    }
    let Some(script) = caller.data().buffer.script else {
        return 0;
    };
    u32::from_be_bytes(script.tag().to_be_bytes()) as i32
}

fn api_buffer_reverse<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, buffer_token: i32) {
    if buffer_token as u32 != BUFFER_TOKEN {
        return;
    }
    let items = caller.data().buffer.len as u64;
    if !charge(&mut caller, items) {
        return;
    }
    caller.data_mut().buffer.reverse();
}

fn api_buffer_reverse_clusters<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    buffer_token: i32,
) {
    if buffer_token as u32 != BUFFER_TOKEN {
        return;
    }
    let items = caller.data().buffer.len as u64;
    if !charge(&mut caller, items) {
        return;
    }
    caller.data_mut().buffer.reverse_clusters();
}

fn direction_to_wasm(direction: Direction) -> u32 {
    match direction {
        Direction::Invalid => DIRECTION_INVALID,
        Direction::LeftToRight => DIRECTION_LTR,
        Direction::RightToLeft => DIRECTION_RTL,
        Direction::TopToBottom => DIRECTION_TTB,
        Direction::BottomToTop => DIRECTION_BTT,
    }
}

// ---------------------------------------------------------------------------
// Fonts and faces
// ---------------------------------------------------------------------------

/// The index of the face or font a guest token names, among those the guest
/// created. The root tokens have none.
fn guest_slot(token: u32) -> Option<usize> {
    usize::try_from(token).ok()?.checked_sub(2)
}

fn face_data(font: &ModelFont, faces: &[Option<WasmFace>], token: u32) -> Option<(Vec<u8>, u32)> {
    if token == ROOT_FACE_TOKEN {
        let (Source::Blob(blob), Kind::Sfnt(_, index)) = (font.source(), font.kind()) else {
            return None;
        };
        Some((blob.as_ref().to_vec(), index))
    } else {
        let face = faces.get(guest_slot(token)?)?.as_ref()?;
        Some((face.data.clone(), face.index))
    }
}

/// The size of a face's data, which `font_create` copies.
fn face_len(font: &ModelFont, faces: &[Option<WasmFace>], token: u32) -> Option<u64> {
    if token == ROOT_FACE_TOKEN {
        let Source::Blob(blob) = font.source() else {
            return None;
        };
        Some(blob.as_ref().len() as u64)
    } else {
        let face = faces
            .get(usize::try_from(token).ok()?.checked_sub(2)?)?
            .as_ref()?;
        Some(face.data.len() as u64)
    }
}

fn with_face_ref<R>(
    font: &ModelFont,
    faces: &[Option<WasmFace>],
    token: u32,
    f: impl FnOnce(FontRef<'_>) -> R,
) -> Option<R> {
    if token == ROOT_FACE_TOKEN {
        let (Source::Blob(blob), Kind::Sfnt(_, index)) = (font.source(), font.kind()) else {
            return None;
        };
        let font_ref = FontRef::from_index(blob, index).ok()?;
        Some(f(font_ref))
    } else {
        let face = faces.get(guest_slot(token)?)?.as_ref()?;
        let font_ref = FontRef::from_index(&face.data, face.index).ok()?;
        Some(f(font_ref))
    }
}

fn api_face_create<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    blob_ptr: i32,
    index: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let blob_ptr = blob_ptr as u32;
    let Some(length) = read_u32(&caller, memory, blob_ptr) else {
        return 0;
    };
    let Some(data_ptr) = read_u32(&caller, memory, blob_ptr + 4) else {
        return 0;
    };
    // The host keeps a copy of the data, outside the module's memory.
    if !caller.data().can_create(u64::from(length))
        || !charge(&mut caller, 4096 + bytes_cost(u64::from(length)))
    {
        return 0;
    }
    let Some(data) = read_bytes(&caller, memory, data_ptr, length) else {
        return 0;
    };
    let index = index as u32;
    if FontRef::from_index(&data, index).is_err() {
        return 0;
    }
    let state = caller.data_mut();
    state.faces.push(Some(WasmFace { data, index }));
    state.object_bytes += u64::from(length);
    i32::try_from(state.faces.len() + 1).unwrap_or(0)
}

fn api_face_destroy<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, face_token: i32) {
    // The root face has no slot, so it cannot be destroyed from here.
    let state = caller.data_mut();
    if let Some(face) = guest_slot(face_token as u32).and_then(|slot| state.faces.get_mut(slot)) {
        *face = None;
    }
}

fn api_face_copy_table<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    face_token: i32,
    table_tag: i32,
    blob_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let tag = RawTag::from_be_bytes((table_tag as u32).to_be_bytes());
    let blob_ptr = blob_ptr as u32;
    let Some(capacity) = read_u32(&caller, memory, blob_ptr) else {
        return 0;
    };
    let Some(data_ptr) = read_u32(&caller, memory, blob_ptr + 4) else {
        return 0;
    };
    let state = caller.data();
    let table_len = with_face_ref(state.font, &state.faces, face_token as u32, |font_ref| {
        font_ref.table_data(tag).map(|data| data.len())
    });
    let Some(Some(table_len)) = table_len else {
        return 0;
    };
    if !charge(&mut caller, bytes_cost(table_len as u64)) {
        return 0;
    }
    let state = caller.data();
    let table = with_face_ref(state.font, &state.faces, face_token as u32, |font_ref| {
        font_ref
            .table_data(tag)
            .map(|data| data.as_bytes().to_vec())
    });
    let Some(Some(table)) = table else {
        return 0;
    };
    let length = u32::try_from(table.len()).unwrap_or(u32::MAX);
    if length <= capacity && (length == 0 || data_ptr != 0) {
        if length > 0 && write_bytes(&mut caller, memory, data_ptr, &table).is_none() {
            return 0;
        }
        return 1;
    }
    let Some(new_ptr) = host_alloc(&mut caller, length, 4) else {
        return 0;
    };
    if write_bytes(&mut caller, memory, new_ptr, &table).is_none() {
        return 0;
    }
    let _ = write_u32(&mut caller, memory, blob_ptr, length);
    let _ = write_u32(&mut caller, memory, blob_ptr + 4, new_ptr);
    1
}

fn api_face_get_upem<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, face_token: i32) -> i32 {
    if !charge(&mut caller, 0) {
        return 0;
    }
    let state = caller.data();
    let upem = with_face_ref(state.font, &state.faces, face_token as u32, |font_ref| {
        font_ref
            .head()
            .ok()
            .map(|head| u32::from(head.units_per_em()))
    });
    upem.flatten()
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or(0)
}

fn root_scale(font: &ShaperFont<'_, '_>) -> (i32, i32) {
    let upem = f32::from(font.units_per_em());
    let scale = font.scale();
    (
        scale.scale_x_unrounded(upem).round() as i32,
        scale.scale_y_unrounded(upem).round() as i32,
    )
}

fn api_font_create<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, face_token: i32) -> i32 {
    // The font gets a copy of the face's data.
    let state = caller.data();
    let Some(length) = face_len(state.font, &state.faces, face_token as u32) else {
        return 0;
    };
    if !state.can_create(length) || !charge(&mut caller, 4096 + bytes_cost(length)) {
        return 0;
    }
    let state = caller.data();
    let Some((data, index)) = face_data(state.font, &state.faces, face_token as u32) else {
        return 0;
    };
    let Some(font) = ModelFont::new(data, index) else {
        return 0;
    };
    let upem = i32::from(font.units_per_em());
    let state = caller.data_mut();
    state.object_bytes += length;
    state.fonts.push(Some(WasmFontEntry {
        face: face_token as u32,
        font,
        x_scale: upem,
        y_scale: upem,
    }));
    i32::try_from(state.fonts.len() + 1).unwrap_or(0)
}

fn api_font_destroy<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, font_token: i32) {
    // The root font has no slot, so it cannot be destroyed from here.
    let state = caller.data_mut();
    if let Some(font) = guest_slot(font_token as u32).and_then(|slot| state.fonts.get_mut(slot)) {
        *font = None;
    }
}

fn api_font_get_face<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, font_token: i32) -> i32 {
    if !charge(&mut caller, 0) {
        return 0;
    }
    if font_token as u32 == ROOT_FONT_TOKEN {
        return ROOT_FACE_TOKEN as i32;
    }
    let state = caller.data_mut();
    let Some(entry) = guest_slot(font_token as u32)
        .and_then(|slot| state.fonts.get_mut(slot))
        .and_then(Option::as_mut)
    else {
        return 0;
    };
    let face_is_live = entry.face == ROOT_FACE_TOKEN
        || guest_slot(entry.face)
            .and_then(|slot| state.faces.get(slot))
            .is_some_and(Option::is_some);
    if !face_is_live {
        // The guest destroyed the face and kept the font. The font has its
        // own copy of the data; give that a new token, as HarfBuzz does.
        let (Source::Blob(blob), Kind::Sfnt(_, index)) = (entry.font.source(), entry.font.kind())
        else {
            return 0;
        };
        state.faces.push(Some(WasmFace {
            data: blob.as_ref().to_vec(),
            index,
        }));
        entry.face = u32::try_from(state.faces.len() + 1).unwrap_or(0);
    }
    i32::try_from(entry.face).unwrap_or(0)
}

fn font_scale(state: &Host<'_, '_, '_>, font_token: u32) -> Option<(i32, i32)> {
    if font_token == ROOT_FONT_TOKEN {
        Some(root_scale(state.font))
    } else {
        state
            .fonts
            .get(guest_slot(font_token)?)?
            .as_ref()
            .map(|entry| (entry.x_scale, entry.y_scale))
    }
}

fn api_font_get_scale<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    x_ptr: i32,
    y_ptr: i32,
) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 0) {
        return;
    }
    let Some((x_scale, y_scale)) = font_scale(caller.data(), font_token as u32) else {
        return;
    };
    let _ = write_i32(&mut caller, memory, x_ptr as u32, x_scale);
    let _ = write_i32(&mut caller, memory, y_ptr as u32, y_scale);
}

fn with_shaper_font<R>(
    state: &Host<'_, '_, '_>,
    font_token: u32,
    f: impl FnOnce(&ShaperFont<'_, '_>) -> R,
) -> Option<R> {
    if font_token == ROOT_FONT_TOKEN {
        Some(f(state.font))
    } else {
        let entry = state.fonts.get(guest_slot(font_token)?)?.as_ref()?;
        let shaper = ShaperFont::new(&entry.font);
        Some(f(&shaper))
    }
}

fn api_font_get_glyph<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    codepoint: i32,
    variation_selector: i32,
) -> i32 {
    if !charge(&mut caller, 0) {
        return 0;
    }
    let state = caller.data();
    let glyph = with_shaper_font(state, font_token as u32, |font| {
        let codepoint = codepoint as u32;
        let selector = variation_selector as u32;
        if selector != 0 {
            font.variation_glyph(codepoint, selector)
                .or_else(|| font.nominal_glyph(codepoint))
        } else {
            font.nominal_glyph(codepoint)
        }
    });
    glyph
        .flatten()
        .and_then(|glyph| i32::try_from(glyph.to_u32()).ok())
        .unwrap_or(0)
}

fn api_font_get_glyph_h_advance<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    glyph: i32,
) -> i32 {
    // May take the variations of the whole glyph.
    if !charge(&mut caller, 1024) {
        return 0;
    }
    with_shaper_font(caller.data(), font_token as u32, |font| {
        font.glyph_h_advance(GlyphId::new(glyph as u32))
    })
    .unwrap_or(0)
}

fn api_font_get_glyph_v_advance<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    glyph: i32,
) -> i32 {
    if !charge(&mut caller, 1024) {
        return 0;
    }
    with_shaper_font(caller.data(), font_token as u32, |font| {
        font.glyph_v_advance(GlyphId::new(glyph as u32))
    })
    .unwrap_or(0)
}

fn api_font_get_glyph_extents<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    glyph: i32,
    extents_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    // May draw the glyph.
    if !charge(&mut caller, 4096) {
        return 0;
    }
    let extents = with_shaper_font(caller.data(), font_token as u32, |font| {
        font.glyph_extents(GlyphId::new(glyph as u32))
    });
    let Some(Some(extents)) = extents else {
        return 0;
    };
    let base = extents_ptr as u32;
    let ok = write_i32(&mut caller, memory, base, extents.x_bearing).is_some()
        && write_i32(&mut caller, memory, base + 4, extents.y_bearing).is_some()
        && write_i32(&mut caller, memory, base + 8, extents.width).is_some()
        && write_i32(&mut caller, memory, base + 12, extents.height).is_some();
    i32::from(ok)
}

fn api_font_glyph_to_string<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    glyph: i32,
    string_ptr: i32,
    size: i32,
) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if size <= 0 || !charge(&mut caller, 256) {
        return;
    }
    let name = with_shaper_font(caller.data(), font_token as u32, |font| {
        font.glyph_name(GlyphId::new(glyph as u32))
    });
    let Some(Some(name)) = name else {
        return;
    };
    let bytes = name.as_str().as_bytes();
    let max = usize::try_from(size as u32 - 1).unwrap_or(0);
    let count = bytes.len().min(max);
    let string_ptr = string_ptr as u32;
    if write_bytes(&mut caller, memory, string_ptr, &bytes[..count]).is_none() {
        return;
    }
    if let Some(addr) = ptr_add(string_ptr, count as u32) {
        let _ = write_bytes(&mut caller, memory, addr, &[0]);
    }
}

fn api_coords_free<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, coords_ptr: i32) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    let coords_ptr = coords_ptr as u32;
    for offset in [0, 4] {
        if let Some(addr) = ptr_add(coords_ptr, offset) {
            let _ = write_u32(&mut caller, memory, addr, 0);
        }
    }
}

fn api_font_copy_coords<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    coords_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let coords_ptr = coords_ptr as u32;
    let Some(capacity) = read_u32(&caller, memory, coords_ptr) else {
        return 0;
    };
    let Some(data_ptr) = read_u32(&caller, memory, coords_ptr + 4) else {
        return 0;
    };
    let coords: Vec<i32> = with_shaper_font(caller.data(), font_token as u32, |font| {
        font.normalized_coords()
            .iter()
            .map(|coord| i32::from(coord.to_bits()))
            .collect()
    })
    .unwrap_or_default();
    let length = u32::try_from(coords.len()).unwrap_or(u32::MAX);
    if !charge(&mut caller, u64::from(length)) {
        return 0;
    }
    if length <= capacity {
        if length > 0 && data_ptr == 0 {
            return 0;
        }
        for (index, coord) in coords.iter().enumerate() {
            if write_i32(&mut caller, memory, data_ptr + index as u32 * 4, *coord).is_none() {
                return 0;
            }
        }
        return 1;
    }
    let Some(new_ptr) = host_alloc(&mut caller, length * 4, 4) else {
        return 0;
    };
    for (index, coord) in coords.iter().enumerate() {
        if write_i32(&mut caller, memory, new_ptr + index as u32 * 4, *coord).is_none() {
            return 0;
        }
    }
    let _ = write_u32(&mut caller, memory, coords_ptr, length);
    let _ = write_u32(&mut caller, memory, coords_ptr + 4, new_ptr);
    1
}

fn api_font_set_coords<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    coords_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let token = font_token as u32;
    if token == ROOT_FONT_TOKEN {
        // The root font is borrowed immutably; a guest that needs a varied
        // font should instantiate a new one from a face.
        return 0;
    }
    let coords_ptr = coords_ptr as u32;
    let Some(length) = read_u32(&caller, memory, coords_ptr) else {
        return 0;
    };
    let Some(data_ptr) = read_u32(&caller, memory, coords_ptr + 4) else {
        return 0;
    };
    let Some(end) = usize::try_from(data_ptr)
        .ok()
        .and_then(|start| start.checked_add(usize::try_from(length).ok()?.checked_mul(4)?))
    else {
        return 0;
    };
    if end > memory.data_size(&caller) {
        return 0;
    }
    // Makes a new instance of the font.
    if !charge(&mut caller, 4096 + u64::from(length)) {
        return 0;
    }
    let mut coords = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    for index in 0..length {
        let Some(value) = read_i32(&caller, memory, data_ptr + index * 4) else {
            return 0;
        };
        coords.push(F2Dot14::from_bits(value as i16));
    }
    let state = caller.data_mut();
    let Some(entry) = guest_slot(token)
        .and_then(|slot| state.fonts.get_mut(slot))
        .and_then(Option::as_mut)
    else {
        return 0;
    };
    entry.font = entry
        .font
        .instance_builder()
        .normalized_coords(coords)
        .build();
    1
}

// ---------------------------------------------------------------------------
// Outlines
// ---------------------------------------------------------------------------

struct RecordingPen<'a> {
    scale: &'a crate::Scale,
    points: Vec<(f32, f32, u32)>,
    contours: Vec<u32>,
}

impl PathSink for RecordingPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.points.push((
            self.scale.scale_x_unrounded(x),
            self.scale.scale_y_unrounded(y),
            MOVE_TO,
        ));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.points.push((
            self.scale.scale_x_unrounded(x),
            self.scale.scale_y_unrounded(y),
            LINE_TO,
        ));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.points.push((
            self.scale.scale_x_unrounded(cx0),
            self.scale.scale_y_unrounded(cy0),
            QUADRATIC_TO,
        ));
        self.points.push((
            self.scale.scale_x_unrounded(x),
            self.scale.scale_y_unrounded(y),
            QUADRATIC_TO,
        ));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.points.push((
            self.scale.scale_x_unrounded(cx0),
            self.scale.scale_y_unrounded(cy0),
            CUBIC_TO,
        ));
        self.points.push((
            self.scale.scale_x_unrounded(cx1),
            self.scale.scale_y_unrounded(cy1),
            CUBIC_TO,
        ));
        self.points.push((
            self.scale.scale_x_unrounded(x),
            self.scale.scale_y_unrounded(y),
            CUBIC_TO,
        ));
    }

    fn close(&mut self) {
        let end = u32::try_from(self.points.len()).unwrap_or(u32::MAX);
        self.contours.push(end);
    }
}

fn api_font_copy_glyph_outline<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    glyph: i32,
    outline_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let outline_ptr = outline_ptr as u32;
    let Some(capacity) = read_u32(&caller, memory, outline_ptr) else {
        return 0;
    };
    let Some(points_ptr) = read_u32(&caller, memory, outline_ptr + 4) else {
        return 0;
    };
    let Some(contour_capacity) = read_u32(&caller, memory, outline_ptr + 8) else {
        return 0;
    };
    let Some(contours_ptr) = read_u32(&caller, memory, outline_ptr + 12) else {
        return 0;
    };
    if !charge(&mut caller, 4096) {
        return 0;
    }

    let recorded = with_shaper_font(caller.data(), font_token as u32, |font| {
        let scale = font.scale();
        let mut pen = RecordingPen {
            scale: &scale,
            points: Vec::new(),
            contours: Vec::new(),
        };
        if let Some(glyph) = font.outline_glyphs().get(GlyphId::new(glyph as u32)) {
            let _ = glyph.draw(&DrawOptions::default(), DrawMemory::Scratch, &mut pen);
        }
        (pen.points, pen.contours)
    });
    let Some((points, contours)) = recorded else {
        return 0;
    };

    let length = u32::try_from(points.len()).unwrap_or(u32::MAX);
    let contour_length = u32::try_from(contours.len()).unwrap_or(u32::MAX);
    // What it took to draw is known only now.
    if !spend(&mut caller, u64::from(length) * 64) {
        return 0;
    }
    let (points_ptr, contours_ptr) =
        if length <= capacity && contour_length <= contour_capacity && points_ptr != 0 {
            (points_ptr, contours_ptr)
        } else {
            let Some(points_bytes) = bytes_for(length, OUTLINE_POINT_SIZE) else {
                return 0;
            };
            let Some(new_points) = host_alloc(&mut caller, points_bytes, 4) else {
                return 0;
            };
            let Some(new_contours) = host_alloc(&mut caller, contour_length * 4, 4) else {
                return 0;
            };
            let _ = write_u32(&mut caller, memory, outline_ptr, length);
            let _ = write_u32(&mut caller, memory, outline_ptr + 4, new_points);
            let _ = write_u32(&mut caller, memory, outline_ptr + 8, contour_length);
            let _ = write_u32(&mut caller, memory, outline_ptr + 12, new_contours);
            (new_points, new_contours)
        };

    for (index, (x, y, kind)) in points.iter().enumerate() {
        let base = points_ptr + index as u32 * OUTLINE_POINT_SIZE;
        if write_f32(&mut caller, memory, base, *x).is_none()
            || write_f32(&mut caller, memory, base + 4, *y).is_none()
            || write_u32(&mut caller, memory, base + 8, *kind).is_none()
        {
            return 0;
        }
    }
    for (index, contour) in contours.iter().enumerate() {
        if write_u32(
            &mut caller,
            memory,
            contours_ptr + index as u32 * 4,
            *contour,
        )
        .is_none()
        {
            return 0;
        }
    }
    1
}

fn api_glyph_outline_free<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, outline_ptr: i32) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 0) {
        return;
    }
    let outline_ptr = outline_ptr as u32;
    for offset in [0, 4, 8, 12] {
        if let Some(addr) = ptr_add(outline_ptr, offset) {
            let _ = write_u32(&mut caller, memory, addr, 0);
        }
    }
}

fn api_blob_free<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, blob_ptr: i32) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 0) {
        return;
    }
    let blob_ptr = blob_ptr as u32;
    for offset in [0, 4] {
        if let Some(addr) = ptr_add(blob_ptr, offset) {
            let _ = write_u32(&mut caller, memory, addr, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// Features and sub-shaping
// ---------------------------------------------------------------------------

fn read_features<T>(
    ctx: impl wasmi::AsContext<Data = T>,
    memory: Memory,
    ptr: u32,
    count: u32,
) -> Option<Vec<Feature>> {
    if count == 0 {
        return Some(Vec::new());
    }
    if ptr == 0 {
        return None;
    }
    let bytes = bytes_for(count, FEATURE_SIZE)?;
    if usize::try_from(ptr)
        .ok()?
        .checked_add(usize::try_from(bytes).ok()?)?
        > memory.data_size(&ctx)
    {
        return None;
    }
    let mut features = Vec::with_capacity(usize::try_from(count).ok()?);
    for index in 0..count {
        let base = ptr + index.checked_mul(FEATURE_SIZE)?;
        let tag = read_u32(&ctx, memory, base)?;
        features.push(Feature {
            tag: Tag::from_be_bytes(tag.to_be_bytes()),
            value: read_u32(&ctx, memory, base + 4)?,
            start: read_u32(&ctx, memory, base + 8)?,
            end: read_u32(&ctx, memory, base + 12)?,
        });
    }
    Some(features)
}

fn api_feature_copy_contents<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    features_ptr: i32,
    count: i32,
    output_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    let output_ptr = output_ptr as u32;
    let count = count as u32;
    if count == 0 {
        let _ = write_u32(&mut caller, memory, output_ptr, 0);
        if let Some(addr) = ptr_add(output_ptr, 4) {
            let _ = write_u32(&mut caller, memory, addr, 0);
        }
        return 1;
    }
    let Some(bytes) = bytes_for(count, FEATURE_SIZE) else {
        return 0;
    };
    if !charge(&mut caller, bytes_cost(u64::from(bytes))) {
        return 0;
    }
    let Some(data) = read_bytes(&caller, memory, features_ptr as u32, bytes) else {
        return 0;
    };
    let Some(new_ptr) = host_alloc(&mut caller, bytes, 4) else {
        return 0;
    };
    if write_bytes(&mut caller, memory, new_ptr, &data).is_none() {
        return 0;
    }
    let _ = write_u32(&mut caller, memory, output_ptr, count);
    if let Some(addr) = ptr_add(output_ptr, 4) {
        let _ = write_u32(&mut caller, memory, addr, new_ptr);
    }
    1
}

fn api_features_free<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, features_ptr: i32) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 0) {
        return;
    }
    let features_ptr = features_ptr as u32;
    for offset in [0, 4] {
        if let Some(addr) = ptr_add(features_ptr, offset) {
            let _ = write_u32(&mut caller, memory, addr, 0);
        }
    }
}

fn api_shape_with<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    font_token: i32,
    buffer_token: i32,
    features_ptr: i32,
    feature_count: i32,
    shaper_ptr: i32,
) -> i32 {
    let Some(memory) = caller.data().memory else {
        return 0;
    };
    if buffer_token as u32 != BUFFER_TOKEN {
        return 0;
    }
    let Some(shaper) = read_cstr(&caller, memory, shaper_ptr as u32) else {
        return 0;
    };
    // The guest's shaper name becomes a one-element list, as in HarfBuzz.
    // "wasm" would re-enter this host, so it is not a shaper the guest may
    // delegate to.
    let shaper = match shaper.as_slice() {
        b"ot" => Shaper::Ot,
        _ => return 0,
    };
    let Some(features) = read_features(&caller, memory, features_ptr as u32, feature_count as u32)
    else {
        return 0;
    };
    // Shaping takes time by the item.
    let items = caller.data().buffer.len as u64;
    if !charge(&mut caller, 4096 + items.saturating_mul(4096)) {
        return 0;
    }

    let state = caller.data_mut();
    let root = state.font;
    let buffer = &mut *state.buffer;
    let options = ShapeOptions::new()
        .features(&features)
        .point_size(state.point_size)
        .shapers(core::slice::from_ref(&shaper));
    if buffer.direction == Direction::Invalid {
        return 0;
    }
    let result = if font_token as u32 == ROOT_FONT_TOKEN {
        crate::shape::shape(root, buffer, options)
    } else {
        let Some(entry) = guest_slot(font_token as u32)
            .and_then(|slot| state.fonts.get(slot))
            .and_then(Option::as_ref)
        else {
            return 0;
        };
        let shaper_font = ShaperFont::new(&entry.font);
        crate::shape::shape(&shaper_font, buffer, options)
    };
    // Shaping under `shape_with` leaves with the default limits, so put the
    // guest call's bounds back before the module runs again.
    buffer.enter();
    i32::from(result.is_ok())
}

// ---------------------------------------------------------------------------
// Script direction
// ---------------------------------------------------------------------------

fn api_script_get_horizontal_direction<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    script: i32,
) -> i32 {
    if !charge(&mut caller, 0) {
        return DIRECTION_INVALID as i32;
    }
    let tag = Tag::from_u32(script as u32);
    let Some(script) = Script::from_iso15924_tag(tag) else {
        return DIRECTION_INVALID as i32;
    };
    match Direction::from_script(script) {
        Some(Direction::RightToLeft) => DIRECTION_RTL as i32,
        Some(Direction::LeftToRight) => DIRECTION_LTR as i32,
        _ => DIRECTION_INVALID as i32,
    }
}

// ---------------------------------------------------------------------------
// Debug output
// ---------------------------------------------------------------------------

fn debug_message<T>(caller: &Caller<'_, T>, memory: Memory, ptr: u32, args: &[i32]) {
    #[cfg(feature = "std")]
    {
        let Some(message) = read_cstr(caller, memory, ptr) else {
            return;
        };
        let mut text = String::from_utf8_lossy(&message).into_owned();
        for arg in args {
            let _ = write!(text, ": {arg}");
        }
        std::eprintln!("[wasm] {text}");
    }
    #[cfg(not(feature = "std"))]
    {
        let _ = (caller, memory, ptr, args);
    }
}

fn api_debugprint<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, ptr: i32) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 1024) {
        return;
    }
    debug_message(&caller, memory, ptr as u32, &[]);
}

fn api_debugprint1<'a, 'b, 'c>(mut caller: Caller<'_, Host<'a, 'b, 'c>>, ptr: i32, arg1: i32) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 1024) {
        return;
    }
    debug_message(&caller, memory, ptr as u32, &[arg1]);
}

fn api_debugprint2<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    ptr: i32,
    arg1: i32,
    arg2: i32,
) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 1024) {
        return;
    }
    debug_message(&caller, memory, ptr as u32, &[arg1, arg2]);
}

fn api_debugprint3<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    ptr: i32,
    arg1: i32,
    arg2: i32,
    arg3: i32,
) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 1024) {
        return;
    }
    debug_message(&caller, memory, ptr as u32, &[arg1, arg2, arg3]);
}

fn api_debugprint4<'a, 'b, 'c>(
    mut caller: Caller<'_, Host<'a, 'b, 'c>>,
    ptr: i32,
    arg1: i32,
    arg2: i32,
    arg3: i32,
    arg4: i32,
) {
    let Some(memory) = caller.data().memory else {
        return;
    };
    if !charge(&mut caller, 1024) {
        return;
    }
    debug_message(&caller, memory, ptr as u32, &[arg1, arg2, arg3, arg4]);
}
