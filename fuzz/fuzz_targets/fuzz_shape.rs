#![no_main]

mod helpers;

use libfuzzer_sys::fuzz_target;

use harfrust::{NormalizedCoord, ShaperData, ShaperInstance, UnicodeBuffer};
use read_fonts::TableProvider;

/// Fixed text used for the first shaping pass, matching hb-shape-fuzzer.cc.
const FIXED_TEXT: &str = "ABCDEXYZ123@_%&)*$!";

/// Extract variable font normalized coordinates from the trailing bytes of the
/// input, matching the technique in HarfBuzz's hb-shape-fuzzer.cc:
///
///   data[size - 1]                  = requested num_coords
///   data[size - 1 - num_coords ..]  = coord bytes, scaled as (byte as i8) * 10
///
/// The full slice is also passed as the font blob; the font parser reads tables
/// by offset and ignores trailing bytes, so this overlap is safe.
fn extract_coords(data: &[u8], axis_count: usize) -> Vec<NormalizedCoord> {
    if data.is_empty() {
        return Vec::new();
    }
    let requested = data[data.len() - 1] as usize;
    let num_coords = requested.min(axis_count);
    if num_coords == 0 || data.len() < num_coords + 1 {
        return Vec::new();
    }
    let coord_bytes = &data[data.len() - 1 - num_coords..data.len() - 1];
    coord_bytes
        .iter()
        .map(|&b| NormalizedCoord::from_bits((b as i8 as i16).wrapping_mul(10)))
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let Ok(font) = helpers::select_font(data) else {
        return;
    };

    let axis_count = font.fvar().map_or(0, |fvar| fvar.axis_count() as usize);
    let coords = extract_coords(data, axis_count);

    let instance = if coords.is_empty() {
        None
    } else {
        Some(ShaperInstance::from_coords(&font, coords))
    };

    let shaper_data = ShaperData::new(&font);
    let shaper = shaper_data
        .shaper(&font)
        .instance(instance.as_ref())
        .build();

    // Pass 1: fixed UTF-8 text with auto-detected segment properties.
    {
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(FIXED_TEXT);
        buffer.guess_segment_properties();
        let _ = shaper.shape(buffer, &[]);
    }

    // Pass 2: last 64 bytes of input reinterpreted as 16 UTF-32 codepoints,
    // matching hb-shape-fuzzer.cc's second hb_buffer_add_utf32 pass.
    {
        let tail = &data[data.len().saturating_sub(64)..];
        let mut buffer = UnicodeBuffer::new();
        for chunk in tail.chunks(4) {
            if chunk.len() == 4 {
                let cp = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if let Some(c) = char::from_u32(cp) {
                    buffer.add(c, 0);
                }
            }
        }
        if !buffer.is_empty() {
            buffer.guess_segment_properties();
            let _ = shaper.shape(buffer, &[]);
        }
    }
});
