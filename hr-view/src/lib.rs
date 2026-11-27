//! Rust implementation of hb-view, rendering text to a PNG image.
//!
//! <https://github.com/harfbuzz/harfbuzz/blob/main/util/hb-view.cc>

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use clap::Parser;
use harfrust::font::glyph::outline::{DrawMemory, DrawOptions, PathSink};
use harfrust::font::{Font, Variation};
use harfrust::{
    shape, Buffer, Direction, Feature, GlyphId, Language, ParseSetting, Script, ShapeOptions,
    Shaper, ShaperFont,
};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// The size text is rendered at when `--font-size` is not given, as in
/// `hb-view`.
const DEFAULT_FONT_SIZE: &str = "256";
/// The border left around the text when `--margin` is not given.
const DEFAULT_MARGIN: f32 = 16.0;
/// Subpixel bits for precision in shaping, as in `hb-view`.
const SUBPIXEL_BITS: i32 = 6;

/// The shapers this build supports, in the order `--list-shapers` prints them.
const SUPPORTED_SHAPERS: &[(&str, Shaper)] = &[
    ("ot", Shaper::Ot),
    #[cfg(feature = "wasm")]
    ("wasm", Shaper::Wasm),
];

fn parse_shaper(value: &str) -> Result<Shaper, String> {
    SUPPORTED_SHAPERS
        .iter()
        .find_map(|&(name, shaper)| (name == value).then_some(shaper))
        .ok_or_else(|| format!("Unknown or unsupported shaper: {value}"))
}

fn parse_font_size(value: &str) -> Result<(i32, i32), String> {
    let (x, y) = value.split_once(',').unwrap_or((value, value));
    let parse = |value: &str| value.parse::<i32>().map_err(|e| e.to_string());
    let (x, y) = (parse(x)?, parse(y)?);
    if x <= 0 || y <= 0 {
        return Err("font size must be positive".to_string());
    }
    Ok((x, y))
}

fn parse_unicodes(s: &str) -> Result<String, String> {
    let mut text = String::new();
    for token in s.split([',', ' ', ';', '\t']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let hex = token
            .strip_prefix("U+")
            .or_else(|| token.strip_prefix("u+"))
            .or_else(|| token.strip_prefix("0x"))
            .or_else(|| token.strip_prefix("0X"))
            .unwrap_or(token);

        let u = u32::from_str_radix(hex, 16)
            .map_err(|_| format!("'{token}' is not a valid codepoint"))?;
        let c = char::try_from(u).map_err(|_| format!("'{token}' is not a valid codepoint"))?;
        text.push(c);
    }
    Ok(text)
}

/// A colour from `rrggbb` or `rrggbbaa`, as `hb-view` takes them.
#[derive(Clone, Copy)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

fn parse_color(spec: &str, allow_none: bool) -> Result<Color, String> {
    let spec = spec.trim();
    if allow_none && spec.eq_ignore_ascii_case("none") {
        return Ok(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        });
    }
    let hex = spec.strip_prefix('#').unwrap_or(spec);
    let byte = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
    };
    match hex.len() {
        6 => Ok(Color {
            r: byte(0).ok_or_else(|| invalid_color(spec))?,
            g: byte(2).ok_or_else(|| invalid_color(spec))?,
            b: byte(4).ok_or_else(|| invalid_color(spec))?,
            a: 255,
        }),
        8 => Ok(Color {
            r: byte(0).ok_or_else(|| invalid_color(spec))?,
            g: byte(2).ok_or_else(|| invalid_color(spec))?,
            b: byte(4).ok_or_else(|| invalid_color(spec))?,
            a: byte(6).ok_or_else(|| invalid_color(spec))?,
        }),
        _ => Err(invalid_color(spec)),
    }
}

fn invalid_color(spec: &str) -> String {
    format!("Invalid color `{spec}`; expected rrggbb or rrggbbaa")
}

impl Color {
    fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }
}

#[derive(Clone, Parser)]
#[command(
    name = "hr-view",
    version,
    about = "View text with a font, as a PNG image"
)]
pub struct Args {
    /// Font file path
    #[arg(value_name = "FONT-FILE")]
    font_file_pos: Option<PathBuf>,

    /// Text to render
    #[arg(value_name = "TEXT")]
    text_pos: Option<String>,

    /// Set font file-name
    #[arg(long)]
    font_file: Option<PathBuf>,

    /// Set face index
    #[arg(short = 'y', long, default_value_t = 0)]
    face_index: u32,

    /// Set font point-size
    #[arg(long)]
    font_ptem: Option<f32>,

    /// Font size in pixels (one value, or separate x,y values)
    #[arg(long, value_parser = parse_font_size, default_value = DEFAULT_FONT_SIZE)]
    font_size: (i32, i32),

    /// Comma-separated list of font variations
    #[arg(long, value_delimiter = ',', value_parser = Variation::parse_setting)]
    variations: Vec<Variation>,

    /// Set named-instance index
    #[arg(long)]
    named_instance: Option<u32>,

    /// Set input text
    #[arg(long)]
    text: Option<String>,

    /// Set input text file-name ("-" for stdin)
    #[arg(long)]
    text_file: Option<PathBuf>,

    /// Set input Unicode codepoints (e.g. 'U+0056,U+0057')
    #[arg(short = 'u', long, value_parser = parse_unicodes)]
    unicodes: Option<String>,

    /// Set text direction (ltr/rtl/ttb/btt)
    #[arg(long)]
    direction: Option<Direction>,

    /// Set text language [default: $LANG]
    #[arg(long)]
    language: Option<Language>,

    /// Set text script as ISO-15924 tag
    #[arg(long)]
    script: Option<Script>,

    /// Comma-separated list of font features
    #[arg(long, value_delimiter = ',')]
    features: Vec<Feature>,

    /// Set shapers to use (default: ot)
    ///
    /// Supported shapers are: ot/wasm
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_shaper,
        default_value = "ot"
    )]
    shapers: Vec<Shaper>,

    /// List available shapers and quit
    #[arg(long)]
    list_shapers: bool,

    /// Set output file-name [default: stdout]
    #[arg(short = 'o', long)]
    output_file: Option<PathBuf>,

    /// Set margin around the text, in pixels
    #[arg(long, default_value_t = DEFAULT_MARGIN)]
    margin: f32,

    /// Set background color (rrggbb or rrggbbaa, or "none")
    #[arg(long, default_value = "ffffff")]
    background: String,

    /// Set foreground color (rrggbb or rrggbbaa)
    #[arg(long, default_value = "000000")]
    foreground: String,

    /// Set extra space between lines, in pixels
    #[arg(long, default_value_t = 0.)]
    line_space: f32,
}

/// The shapers this build supports, one per line, as `--list-shapers` prints
/// them.
pub fn shaper_list() -> String {
    let mut list = String::new();
    for &(name, _) in SUPPORTED_SHAPERS {
        list.push_str(name);
        list.push('\n');
    }
    list
}

/// Checks if the terminal supports inline image display.
///
/// Supports iTerm2, WezTerm (via iTerm2 protocol), and Kitty graphics protocol.
fn terminal_image_protocol() -> Option<&'static str> {
    if !io::stdout().is_terminal() {
        return None;
    }

    // Check for iTerm2 (via LC_TERMINAL environment variable)
    if let Ok(term) = std::env::var("LC_TERMINAL") {
        if term.eq_ignore_ascii_case("iTerm2") {
            return Some("iterm2");
        }
    }

    // Check for WezTerm (via TERM_PROGRAM environment variable)
    if let Ok(term) = std::env::var("TERM_PROGRAM") {
        if term.eq_ignore_ascii_case("WezTerm") {
            return Some("iterm2");
        }
    }

    // Check for Kitty terminal
    if let Ok(term) = std::env::var("TERM") {
        if term == "xterm-kitty" {
            return Some("kitty");
        }
    }

    None
}

/// Writes an inline image to the terminal using the appropriate protocol.
fn write_terminal_image(png: &[u8], protocol: &str) -> Result<(), String> {
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
    let output = match protocol {
        "iterm2" => format!(
            "\x1b]1337;File=inline=1;size={}:{}\x07\n",
            encoded.len(),
            encoded
        ),
        "kitty" => {
            // Kitty protocol: https://sw.kovidgoyal.net/kitty/graphics-protocol.html
            const CHUNK_SIZE: usize = 4096;
            let mut result = String::new();
            for (i, chunk) in encoded.as_bytes().chunks(CHUNK_SIZE).enumerate() {
                if i == 0 {
                    result.push_str("\x1b_Ga=T,f=100,m=");
                } else {
                    result.push_str("\x1b_Gm=");
                }
                result.push_str(&String::from_utf8_lossy(chunk));
                result.push_str("\x1b\\\n");
            }
            result
        }
        _ => return Err(format!("Unknown terminal protocol: {protocol}")),
    };

    io::stdout()
        .lock()
        .write_all(output.as_bytes())
        .map_err(|e| format!("Error: writing to terminal: {e}"))
}

/// Parses command-line arguments and renders the text to a file.
///
/// # Errors
///
/// Returns an error string if argument parsing, rendering, or writing the
/// output fails.
pub fn try_main() -> Result<(), String> {
    run(Args::parse())
}

/// Renders the text described by `args` and writes the PNG where it asks.
///
/// With `--list-shapers`, prints the shapers this build supports instead.
///
/// # Errors
///
/// Returns an error string if the font or text cannot be read, shaping fails,
/// or the output cannot be written.
pub fn run(args: Args) -> Result<(), String> {
    if args.list_shapers {
        return io::stdout()
            .lock()
            .write_all(shaper_list().as_bytes())
            .map_err(|e| format!("Error: writing stdout: {e}"));
    }
    let png = render_png(&args)?;
    if let Some(path) = args.output_file.clone() {
        if path.to_string_lossy() == "-" {
            io::stdout()
                .lock()
                .write_all(&png)
                .map_err(|e| format!("Error: writing stdout: {e}"))
        } else {
            write_output(&png, &path)
        }
    } else if let Some(protocol) = terminal_image_protocol() {
        write_terminal_image(&png, protocol)
    } else {
        io::stdout()
            .lock()
            .write_all(&png)
            .map_err(|e| format!("Error: writing stdout: {e}"))
    }
}

/// Parses `hr-view` arguments and returns the PNG image.
///
/// If `-o/--output-file` is present, this also writes the image there.
///
/// # Errors
///
/// Returns an error string if argument parsing, rendering, or writing the
/// output fails.
pub fn run_from_args<I, T>(args: I) -> Result<Vec<u8>, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::try_parse_from(args).map_err(|e| e.to_string())?;
    let png = render_png(&args)?;
    if let Some(path) = args.output_file.as_ref() {
        write_output(&png, path)?;
    }
    Ok(png)
}

/// Renders the text described by `args` and encodes it as a PNG.
///
/// # Errors
///
/// Returns an error string if the font or text cannot be read or shaping
/// fails.
pub fn render_png(args: &Args) -> Result<Vec<u8>, String> {
    render(args)?
        .encode_png()
        .map_err(|e| format!("Error: cannot encode PNG: {e}"))
}

fn write_output(png: &[u8], path: &PathBuf) -> Result<(), String> {
    std::fs::write(path, png).map_err(|e| format!("Error: cannot write '{}': {e}", path.display()))
}

/// A glyph placed relative to its line's origin, in pixels.
struct PlacedGlyph {
    glyph: GlyphId,
    x: f32,
    y: f32,
}

/// One shaped line: its glyphs and how far the pen moved across the line.
struct Line {
    glyphs: Vec<PlacedGlyph>,
    advance: (f32, f32),
}

/// Renders the text described by `args` to an image.
///
/// # Errors
///
/// Returns an error string if the font or text cannot be read or shaping
/// fails.
pub fn render(args: &Args) -> Result<Pixmap, String> {
    let background = parse_color(&args.background, true)?;
    let foreground = parse_color(&args.foreground, false)?;
    let (font_path, text) = resolve(args)?;

    let font_data = std::fs::read(&font_path)
        .map_err(|e| format!("Error: cannot read '{}': {e}", font_path.display()))?;
    let font = Font::new(font_data, args.face_index)
        .ok_or_else(|| format!("Error: face index {} not found.", args.face_index))?;

    let variations = args.variations.iter().map(|v| (v.tag, v.value));
    let builder = font.instance_builder();
    let instance = match args.named_instance {
        Some(index) => builder
            .named_instance_with_overrides(index, variations)
            .build(),
        None => builder.variations(variations).build(),
    };
    let mut shaping_font = ShaperFont::new(&instance);
    let subpixel_factor = 1 << SUBPIXEL_BITS; // 64
    shaping_font.set_scale_separate(
        args.font_size.0 * subpixel_factor,
        args.font_size.1 * subpixel_factor,
    );

    let mut direction = args.direction;
    let mut buffer = Buffer::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        buffer.clear();
        buffer.push_str(line);
        if let Some(direction) = args.direction {
            buffer.set_direction(direction);
        }
        if let Some(language) = &args.language {
            buffer.set_language(Some(language.clone()));
        }
        if let Some(script) = args.script {
            buffer.set_script(Some(script));
        }
        buffer.guess_segment_properties();
        shape(
            &shaping_font,
            &mut buffer,
            ShapeOptions::new()
                .features(&args.features)
                .point_size(args.font_ptem)
                .shapers(&args.shapers),
        )
        .map_err(|e| format!("Error: {e}"))?;
        if direction.is_none() && buffer.direction() != Direction::Invalid {
            direction = Some(buffer.direction());
        }

        let mut pen = (0.0f32, 0.0f32);
        let mut glyphs = Vec::with_capacity(buffer.glyph_infos().len());
        for (info, position) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
            glyphs.push(PlacedGlyph {
                glyph: GlyphId::new(info.glyph_id),
                x: pen.0 + px(position.x_offset),
                y: pen.1 + px(position.y_offset),
            });
            pen.0 += px(position.x_advance);
            pen.1 += px(position.y_advance);
        }
        lines.push(Line {
            glyphs,
            advance: pen,
        });
    }
    if lines.is_empty() {
        lines.push(Line {
            glyphs: Vec::new(),
            advance: (0.0, 0.0),
        });
    }

    let vertical = matches!(
        direction,
        Some(Direction::TopToBottom | Direction::BottomToTop)
    );
    let metrics = shaping_font.metrics();
    let upem = f32::from(metrics.units_per_em).max(1.0);
    let x_mult = (args.font_size.0 as f64 / upem as f64) as f32;
    let y_mult = (args.font_size.1 as f64 / upem as f64) as f32;
    let line_box = if vertical {
        metrics.vhea_line.or(metrics.hhea_line)
    } else if metrics.use_typo_metrics {
        metrics.typo_line.or(metrics.hhea_line)
    } else {
        metrics.hhea_line.or(metrics.typo_line)
    };
    // Vertical line metrics are measured across the baseline, so they scale
    // on x rather than y.
    let across = if vertical { x_mult } else { y_mult };
    let (ascent, descent, line_gap) =
        line_box.map_or((0.8 * upem * across, 0.2 * upem * across, 0.0), |line| {
            (
                line.ascender.to_f32() * across,
                -line.descender.to_f32() * across,
                line.line_gap.to_f32() * across,
            )
        });
    let leading = ascent + descent + line_gap + args.line_space;

    // The box the text occupies before any glyph is drawn, as hb-view
    // computes it: the advance across the lines by the number of lines.
    let line_count = px_of_usize(lines.len());
    let mut max_advance = 0.0f32;
    for line in &lines {
        let advance = if vertical {
            -line.advance.1
        } else {
            line.advance.0
        };
        max_advance = max_advance.max(advance);
    }
    let (mut min_x, mut max_x, mut min_y, mut max_y) = if vertical {
        (
            -((line_count - 1.0) * leading + ascent),
            descent,
            -max_advance.max(0.0),
            0.0,
        )
    } else {
        let height = line_count * leading - (line_gap + args.line_space);
        (0.0, max_advance.max(0.0), ascent - height, ascent)
    };

    let outlines = shaping_font.outline_glyphs();
    let mut paths = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        // hb-view stacks lines downward when horizontal and right to left
        // when vertical.
        let (origin_x, origin_y) = if vertical {
            (-px_of_usize(index) * leading, 0.0)
        } else {
            (0.0, -px_of_usize(index) * leading)
        };
        for placed in &line.glyphs {
            let Some(outline) = outlines.get(placed.glyph) else {
                continue;
            };
            let mut builder = PathBuilder::new();
            let mut pen = OutlinePen {
                builder: &mut builder,
                x_mult,
                y_mult,
                dx: origin_x + placed.x,
                dy: origin_y + placed.y,
            };
            let _ = outline.draw(&DrawOptions::default(), DrawMemory::Scratch, &mut pen);
            let Some(path) = builder.finish() else {
                continue;
            };
            let bounds = path.bounds();
            min_x = min_x.min(bounds.left());
            max_x = max_x.max(bounds.right());
            min_y = min_y.min(bounds.top());
            max_y = max_y.max(bounds.bottom());
            paths.push(path);
        }
    }

    let width = (max_x - min_x + 2.0 * args.margin).ceil().max(1.0);
    let height = (max_y - min_y + 2.0 * args.margin).ceil().max(1.0);
    if width > 16_384.0 || height > 16_384.0 {
        return Err(format!(
            "Error: the image would be {width} by {height} pixels; try a smaller font size"
        ));
    }
    let mut pixmap = Pixmap::new(pixel_count(width), pixel_count(height))
        .ok_or_else(|| "Error: cannot allocate the image".to_string())?;
    pixmap.fill(background.to_skia());

    let mut paint = Paint::default();
    paint.set_color(foreground.to_skia());
    paint.anti_alias = true;
    // Glyph outlines have y up and the image has y down.
    let transform = Transform::from_row(
        1.0,
        0.0,
        0.0,
        -1.0,
        args.margin - min_x,
        args.margin + max_y,
    );
    for path in &paths {
        pixmap.fill_path(path, &paint, FillRule::Winding, transform, None);
    }
    Ok(pixmap)
}

/// Draws a glyph's outline into a path, in the position the shaper put it.
struct OutlinePen<'a> {
    builder: &'a mut PathBuilder,
    x_mult: f32,
    y_mult: f32,
    dx: f32,
    dy: f32,
}

impl PathSink for OutlinePen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder
            .move_to(x * self.x_mult + self.dx, y * self.y_mult + self.dy);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder
            .line_to(x * self.x_mult + self.dx, y * self.y_mult + self.dy);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.builder.quad_to(
            cx0 * self.x_mult + self.dx,
            cy0 * self.y_mult + self.dy,
            x * self.x_mult + self.dx,
            y * self.y_mult + self.dy,
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.builder.cubic_to(
            cx0 * self.x_mult + self.dx,
            cy0 * self.y_mult + self.dy,
            cx1 * self.x_mult + self.dx,
            cy1 * self.y_mult + self.dy,
            x * self.x_mult + self.dx,
            y * self.y_mult + self.dy,
        );
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

#[allow(clippy::cast_precision_loss)]
fn px(value: i32) -> f32 {
    value as f32 / 64.0
}

#[allow(clippy::cast_precision_loss)]
fn px_of_usize(value: usize) -> f32 {
    value as f32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pixel_count(value: f32) -> u32 {
    value as u32
}

/// Returns the font path and the text to render.
fn resolve(args: &Args) -> Result<(PathBuf, String), String> {
    let mut font_set_as_free_arg = false;
    let font_path = if let Some(path) = &args.font_file {
        path.clone()
    } else if let Some(path) = &args.font_file_pos {
        font_set_as_free_arg = true;
        path.clone()
    } else {
        return Err("Error: font is not set.".to_string());
    };
    if !font_path.exists() {
        return Err(format!("Error: '{}' does not exist.", font_path.display()));
    }

    let text = if let Some(path) = &args.text_file {
        if path == &PathBuf::from("-") {
            read_stdin()?
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| format!("Error: cannot read '{}': {e}", path.display()))?
        }
    } else if font_set_as_free_arg {
        match (&args.text_pos, &args.unicodes, &args.text) {
            (Some(text), _, _) => text.clone(),
            (_, Some(unicodes), _) => unicodes.clone(),
            (_, _, Some(text)) => text.clone(),
            _ => read_stdin()?,
        }
    } else if let Some(text) = &args.font_file_pos {
        text.to_string_lossy().to_string()
    } else if let Some(unicodes) = &args.unicodes {
        unicodes.clone()
    } else if let Some(text) = &args.text {
        text.clone()
    } else {
        read_stdin()?
    };
    Ok((font_path, text))
}

fn read_stdin() -> Result<String, String> {
    io::read_to_string(io::stdin()).map_err(|e| format!("Error: reading stdin: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROBOTO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../harfrust/benches/fonts/Roboto-Regular.ttf"
    );

    fn args(values: &[&str]) -> Args {
        Args::try_parse_from(values).unwrap()
    }

    #[test]
    fn renders_text_as_an_image() {
        let pixmap = render(&args(&["hr-view", ROBOTO, "Hello"])).unwrap();
        assert!(pixmap.width() > 100);
        assert!(pixmap.height() > 100);
        // The background is white and the glyphs are drawn in black.
        assert_eq!(pixmap.pixel(0, 0).unwrap().red(), 255);
        assert!(pixmap.pixels().iter().any(|pixel| pixel.red() < 128));
    }

    #[test]
    fn font_size_changes_the_image_size() {
        let small = render(&args(&["hr-view", ROBOTO, "Hello", "--font-size=32"])).unwrap();
        let large = render(&args(&["hr-view", ROBOTO, "Hello", "--font-size=128"])).unwrap();
        assert!(large.width() > small.width());
        assert!(large.height() > small.height());
    }

    #[test]
    fn uses_the_requested_colors() {
        let pixmap = render(&args(&[
            "hr-view",
            ROBOTO,
            "Hello",
            "--background=000000",
            "--foreground=ffffff",
        ]))
        .unwrap();
        let corner = pixmap.pixel(0, 0).unwrap();
        assert_eq!(
            (corner.red(), corner.green(), corner.blue(), corner.alpha()),
            (0, 0, 0, 255)
        );
        assert!(pixmap
            .pixels()
            .iter()
            .any(|pixel| pixel.red() == 255 && pixel.green() == 255 && pixel.blue() == 255));
    }

    #[test]
    fn a_background_of_none_is_transparent() {
        let pixmap = render(&args(&["hr-view", ROBOTO, "Hello", "--background=none"])).unwrap();
        assert_eq!(pixmap.pixel(0, 0).unwrap().alpha(), 0);
    }

    #[test]
    fn an_invalid_color_is_rejected() {
        let error =
            render(&args(&["hr-view", ROBOTO, "Hello", "--foreground=zzzzzz"])).unwrap_err();
        assert!(error.contains("Invalid color `zzzzzz`"));
    }

    #[test]
    fn writes_the_png_it_is_asked_for() {
        let directory = std::env::temp_dir().join("hr-view-writes-the-png-it-is-asked-for");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("out.png");
        let png = run_from_args([
            "hr-view",
            ROBOTO,
            "Hi",
            "-o",
            path.to_str().unwrap(),
            "--font-size=32",
        ])
        .unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(std::fs::read(&path).unwrap(), png);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn lines_are_stacked() {
        let one = render(&args(&["hr-view", ROBOTO, "one", "--font-size=32"])).unwrap();
        let two = render(&args(&["hr-view", ROBOTO, "one\ntwo", "--font-size=32"])).unwrap();
        assert!(two.height() > one.height());
        assert!(two.width() >= one.width());
    }

    #[test]
    fn lists_the_shapers_it_supports() {
        assert!(shaper_list().starts_with("ot\n"));
    }

    #[test]
    fn rejects_an_unknown_shaper() {
        let error = Args::try_parse_from(["hr-view", ROBOTO, "Hi", "--shapers=ot,wat"])
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("Unknown or unsupported shaper: wat"));
    }

    #[test]
    fn a_missing_font_is_an_error() {
        let error = render(&args(&["hr-view", "no-such-font.ttf", "Hi"])).unwrap_err();
        assert!(error.contains("does not exist"));
    }

    #[cfg(feature = "wasm")]
    #[test]
    fn renders_with_the_fonts_wasm_module() {
        let font = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../harfrust/tests/fonts/wasm/wasm-api.ttf"
        );
        let pixmap = render(&args(&["hr-view", font, "a", "--shapers=wasm"])).unwrap();
        assert!(pixmap.width() > 0 && pixmap.height() > 0);

        // Roboto has no `Wasm` table, so naming only that shaper fails, and
        // naming OpenType after it renders the font anyway.
        let error = render(&args(&["hr-view", ROBOTO, "a", "--shapers=wasm"])).unwrap_err();
        assert!(error.contains("no requested shaper"));
        render(&args(&["hr-view", ROBOTO, "a", "--shapers=wasm,ot"])).unwrap();
    }
}
