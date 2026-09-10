//! Every glyph and position this build produces, for diffing one build's
//! output against another's. Kept out of the repository: a correctness check
//! for the compiled path, not a part of the crate.

use harfrust::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(font_path), Some(text_path)) = (args.next(), args.next()) else {
        eprintln!("usage: hrdump <font> <text>");
        std::process::exit(2);
    };
    let data = std::fs::read(&font_path).expect("font");
    let text = std::fs::read_to_string(&text_path).expect("text");
    let font = FontRef::from_index(&data, 0).unwrap();
    let shaper_data = ShaperData::new(&font);
    let shaper = shaper_data.shaper(&font).build();
    let mut held = Some(UnicodeBuffer::new());
    let mut out = String::new();
    for (n, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let mut buffer = held.take().unwrap();
        buffer.push_str(line);
        buffer.guess_segment_properties();
        let g = shaper.shape(buffer, ShapeOptions::new());
        out.push_str(&format!("{n}:"));
        for (i, p) in g.glyph_infos().iter().zip(g.glyph_positions()) {
            out.push_str(&format!(
                " {}/{}/{},{},{},{}",
                i.glyph_id, i.cluster, p.x_advance, p.y_advance, p.x_offset, p.y_offset
            ));
        }
        out.push('\n');
        held = Some(g.clear());
    }
    print!("{out}");
}
