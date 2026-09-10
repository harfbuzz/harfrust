#![allow(
    unused_mut,
    clippy::single_match_else,
    clippy::match_wildcard_for_single_variants
)]
//! Shaping throughput, for comparing builds of this crate against each other
//! and against `hbbench.c`.
//!
//! Structure mirrors what a caller writes, and what the C side does: face and
//! font built once outside the timed loop, a plan cache keyed by direction and
//! script, one buffer reused across lines.
//!
//! Timing scales the work to the clock rather than fixing a sample count. An
//! earlier version took twenty samples of one pass each, which gave a case
//! shaping ten thousand short words a five to eight percent spread while a case
//! shaping a long document sat at one percent -- the short case's sample was
//! only eight milliseconds, so a single scheduler interruption moved the
//! median. Every sample now covers about the same wall time, so every case is
//! measured to about the same precision, and the spread is printed so a run
//! that went badly says so.

use harfrust::{FontRef, ShapeOptions, ShapePlan, ShapePlanKey, ShaperData, UnicodeBuffer};
use std::time::{Duration, Instant};

/// Wall time each sample should cover.
const TARGET: Duration = Duration::from_millis(400);
const SAMPLES: usize = 15;
/// How long to run untimed first: plans built, lookups compiled, caches
/// filled, clock ramped.
const WARMUP: Duration = Duration::from_millis(600);

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(font_path), Some(text_path)) = (args.next(), args.next()) else {
        eprintln!("usage: hrbench <font> <text>");
        std::process::exit(2);
    };
    let data = std::fs::read(&font_path).expect("font");
    let text = std::fs::read_to_string(&text_path).expect("text");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();

    let font = FontRef::from_index(&data, 0).unwrap();
    let shaper_data = ShaperData::new(&font);
    let shaper = shaper_data.shaper(&font).build();
    let mut plans: Vec<ShapePlan> = Vec::new();
    let mut glyphs = 0usize;

    let mut pass = |plans: &mut Vec<ShapePlan>, count: &mut usize| {
        let mut held = Some(UnicodeBuffer::new());
        for line in &lines {
            let mut buffer = held.take().unwrap();
            buffer.push_str(line);
            buffer.guess_segment_properties();
            let key = ShapePlanKey::new(Some(buffer.script()), buffer.direction());
            let idx = match plans.iter().position(|p| key.matches(p)) {
                Some(i) => i,
                None => {
                    plans.push(ShapePlan::new(
                        &shaper,
                        buffer.direction(),
                        Some(buffer.script()),
                        None,
                        &[],
                    ));
                    plans.len() - 1
                }
            };
            let out = shaper.shape(buffer, ShapeOptions::new().plan(Some(&plans[idx])));
            *count += out.len();
            held = Some(out.clear());
        }
    };

    // One pass to price the work, then warm for a fixed duration.
    let probe = Instant::now();
    pass(&mut plans, &mut glyphs);
    let one = probe.elapsed().max(Duration::from_nanos(1));
    let shaped = glyphs;

    let warm = Instant::now();
    while warm.elapsed() < WARMUP {
        pass(&mut plans, &mut glyphs);
    }

    // Enough passes per sample that the clock's resolution and any single
    // interruption are small against it.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let inner = (TARGET.as_secs_f64() / one.as_secs_f64()).ceil().max(1.0) as usize;
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..inner {
            pass(&mut plans, &mut glyphs);
        }
        #[allow(clippy::cast_precision_loss)]
        let per = start.elapsed().as_secs_f64() / inner as f64;
        samples.push(per);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[SAMPLES / 2] * 1000.0;
    let spread = (samples[SAMPLES - 1] - samples[0]) / samples[SAMPLES / 2] * 100.0;
    println!(
        "{median:.4} ms  ({} lines, {shaped} glyphs, {inner}x/sample, spread {spread:.1}%)",
        lines.len(),
    );

    // With counters compiled in the timings are meaningless, so report what
    // the run actually did instead.
    #[cfg(feature = "compile-stats")]
    {
        let per = |n: u64| n as f64 / shaped as f64;
        println!("  per shaped glyph, over the whole run:");
        for (name, n) in harfrust::_compile_stats() {
            println!("    {name:<12} {n:>14}  {:>8.2} / glyph", per(n));
        }
    }
}
