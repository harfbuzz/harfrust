//! What the first shape on a fresh face costs.
//!
//! Steady-state throughput is what the other harness measures, and it is not
//! what a caller shaping one line of one document feels. Everything the
//! compiled path builds is built lazily on the way through the first shape, so
//! the gap between the first call and the ones after it is the price of
//! compiling whatever that call reached.

use harfrust::{FontRef, ShapeOptions, ShaperData, UnicodeBuffer};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(font_path), Some(text_path)) = (args.next(), args.next()) else {
        eprintln!("usage: hrcold <font> <text>");
        std::process::exit(2);
    };
    let data = std::fs::read(&font_path).expect("font");
    let text = std::fs::read_to_string(&text_path).expect("text");
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");

    const RUNS: usize = 200;
    let mut first = Vec::with_capacity(RUNS);
    let mut warm = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        // A fresh face and shaper each time: nothing is compiled yet.
        let font = FontRef::from_index(&data, 0).unwrap();
        let shaper_data = ShaperData::new(&font);
        let shaper = shaper_data.shaper(&font).build();

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(line);
        buffer.guess_segment_properties();
        let start = Instant::now();
        let out = shaper.shape(buffer, ShapeOptions::new());
        first.push(start.elapsed().as_secs_f64() * 1e6);

        let mut buffer = out.clear();
        buffer.push_str(line);
        buffer.guess_segment_properties();
        let start = Instant::now();
        let out = shaper.shape(buffer, ShapeOptions::new());
        warm.push(start.elapsed().as_secs_f64() * 1e6);
        let _ = out;
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (f, w) = (median(&mut first), median(&mut warm));
    println!(
        "first {f:9.1} us   warm {w:9.1} us   compile {:9.1} us",
        f - w
    );
}
