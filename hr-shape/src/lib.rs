//! Rust implementation of hb-shape.
//! <https://github.com/harfbuzz/harfbuzz/blob/main/util/hb-shape.cc>

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::Parser;
use harfrust::{
    BufferClusterLevel, BufferFlags, Direction, Feature, FontRef, Language, SerializeFlags,
    ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};

#[derive(Clone, Parser)]
#[command(name = "hr-shape", version, about = "Shape text using HarfRust")]
pub struct Args {
    /// Font file path
    #[arg(value_name = "FONT-FILE")]
    font_file_pos: Option<PathBuf>,

    /// Text to shape
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

    /// Comma-separated list of font variations
    #[arg(long, value_delimiter = ',')]
    variations: Vec<Variation>,

    /// Set named-instance index
    #[arg(long)]
    named_instance: Option<usize>,

    /// Set input text
    #[arg(long)]
    text: Option<String>,

    /// Set input text file-name ("-" for stdin)
    #[arg(long)]
    text_file: Option<PathBuf>,

    /// Set input Unicode codepoints (e.g. 'U+0056,U+0057')
    #[arg(short = 'u', long, value_parser = parse_unicodes)]
    unicodes: Option<String>,

    /// Set text context before each line
    #[arg(long)]
    text_before: Option<String>,

    /// Set text context after each line
    #[arg(long)]
    text_after: Option<String>,

    /// Set Unicode codepoints context before each line
    #[arg(long, value_parser = parse_unicodes)]
    unicodes_before: Option<String>,

    /// Set Unicode codepoints context after each line
    #[arg(long, value_parser = parse_unicodes)]
    unicodes_after: Option<String>,

    /// Treat text as single paragraph
    #[arg(long)]
    single_par: bool,

    /// Set text direction (ltr/rtl/ttb/btt)
    #[arg(long)]
    direction: Option<Direction>,

    /// Set text language [default: $LANG]
    #[arg(long)]
    language: Option<Language>,

    /// Set text script as ISO-15924 tag
    #[arg(long)]
    script: Option<harfrust::Script>,

    /// Comma-separated list of font features
    #[arg(long, value_delimiter = ',')]
    features: Vec<Feature>,

    /// Use UTF-8 byte indices, not char indices
    #[arg(long)]
    utf8_clusters: bool,

    /// Cluster merging level (0-3)
    #[arg(long, value_parser = parse_cluster, default_value = "0")]
    cluster_level: BufferClusterLevel,

    /// Treat text as beginning of paragraph
    #[arg(long)]
    bot: bool,

    /// Treat text as end of paragraph
    #[arg(long)]
    eot: bool,

    /// Preserve Default-Ignorable characters
    #[arg(long)]
    preserve_default_ignorables: bool,

    /// Remove Default-Ignorable characters
    #[arg(long)]
    remove_default_ignorables: bool,

    /// Glyph value to replace not-found variation-selector characters with
    #[arg(long)]
    not_found_variation_selector_glyph: Option<u32>,

    /// Produce unsafe-to-concat glyph flag
    #[arg(long)]
    unsafe_to_concat: bool,

    /// Produce safe-to-insert-tatweel glyph flag
    #[arg(long)]
    safe_to_insert_tatweel: bool,

    /// Perform sanity checks on shaping results
    #[arg(long)]
    verify: bool,

    /// Prefix each line of output with its input text
    #[arg(long)]
    show_text: bool,

    /// Prefix each line of output with its input codepoints
    #[arg(long)]
    show_unicode: bool,

    /// Prefix each line of output with its line number
    #[arg(long)]
    show_line_num: bool,

    /// Prefix each line of output with text, unicode, and line number
    #[arg(long)]
    verbose: bool,

    /// Shorthand for --verbose --ned (matching hb-shape behavior)
    #[arg(short = 'v', hide = true)]
    short_v: bool,

    /// Output glyph indices instead of names
    #[arg(long)]
    no_glyph_names: bool,

    /// Do not output glyph positions
    #[arg(long)]
    no_positions: bool,

    /// Do not output glyph advances
    #[arg(long)]
    no_advances: bool,

    /// Do not output cluster indices
    #[arg(long)]
    no_clusters: bool,

    /// Output glyph extents
    #[arg(long)]
    show_extents: bool,

    /// Output glyph flags
    #[arg(long)]
    show_flags: bool,

    /// No Extra Data; Do not output clusters or advances
    #[arg(long)]
    ned: bool,

    /// Set output file-name [default: stdout]
    #[arg(short = 'o', long)]
    output_file: Option<PathBuf>,

    /// Run shaper N times
    #[arg(short = 'n', long, default_value_t = 1)]
    num_iterations: u32,

    /// Ignored; accepted for hb-shape compatibility
    #[arg(long, hide = true)]
    font_funcs: Option<String>,
}

/// Parses command-line arguments and runs the `hr-shape` command.
///
/// # Errors
///
/// Returns an error string if argument parsing, shaping, or output writing fails.
pub fn try_main() -> Result<(), String> {
    let args = Args::parse();
    run_and_write(args)
}

/// Runs `hr-shape` from a parsed argument struct and writes output to the configured destination.
///
/// # Errors
///
/// Returns an error string if shaping or output writing fails.
pub fn run_and_write(args: Args) -> Result<(), String> {
    let output_file = args.output_file.clone();
    let output = render(args)?;
    write_output(&output, output_file.as_ref())?;
    Ok(())
}

/// Parses `hr-shape` arguments from an iterator and returns the rendered output.
///
/// If `-o/--output-file` is present, this also writes the rendered output to that file.
///
/// # Errors
///
/// Returns an error string if argument parsing, shaping, or requested file output fails.
pub fn run_from_args<I, T>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::try_parse_from(args).map_err(|e| e.to_string())?;
    let output = render(args.clone())?;
    if let Some(path) = args.output_file.as_ref() {
        write_output(&output, Some(path))?;
    }
    Ok(output)
}

/// Shapes a single input string using the same option parsing path as the CLI.
///
/// The input text is passed as Unicode codepoints so tests can include NUL bytes and other
/// special characters without shell escaping concerns.
///
/// # Errors
///
/// Returns an error string if option parsing or shaping fails.
pub fn shape(font_path: &str, text: &str, options: &str) -> Result<String, String> {
    let unicodes: Vec<String> = text
        .chars()
        .map(|c| format!("U+{:04X}", c as u32))
        .collect();
    let mut args = vec![
        "hr-shape".to_string(),
        "--font-file".to_string(),
        font_path.to_string(),
        "-u".to_string(),
        unicodes.join(","),
        "--single-par".to_string(),
    ];
    args.extend(
        options
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned),
    );
    run_from_args(args)
}

/// Renders `hr-shape` output for a parsed argument struct without writing to stdout.
///
/// # Errors
///
/// Returns an error string if font loading, input loading, or shaping fails.
pub fn render(mut args: Args) -> Result<String, String> {
    normalize_args(&mut args);

    let mut font_set_as_free_arg = false;
    let font_path = if let Some(ref path) = args.font_file {
        path.clone()
    } else if let Some(ref path) = args.font_file_pos {
        font_set_as_free_arg = true;
        path.clone()
    } else {
        return Err("Error: font is not set.".to_string());
    };

    if !font_path.exists() {
        return Err(format!("Error: '{}' does not exist.", font_path.display()));
    }

    let font_data = std::fs::read(&font_path)
        .map_err(|e| format!("Error: cannot read '{}': {e}", font_path.display()))?;
    let font = FontRef::from_index(&font_data, args.face_index)
        .map_err(|_| format!("Error: face index {} not found.", args.face_index))?;

    let data = ShaperData::new(&font);
    let variations = &args.variations;
    let instance = match args.named_instance {
        Some(idx) => {
            let mut inst = ShaperInstance::from_named_instance(&font, idx);
            if !variations.is_empty() {
                inst.set_variations(&font, variations);
            }
            inst
        }
        None => ShaperInstance::from_variations(&font, variations),
    };
    let shaper = data
        .shaper(&font)
        .instance(Some(&instance))
        .point_size(args.font_ptem)
        .build();

    let pre_context = args
        .unicodes_before
        .as_deref()
        .or(args.text_before.as_deref());
    let post_context = args
        .unicodes_after
        .as_deref()
        .or(args.text_after.as_deref());

    let mut buf_flags = BufferFlags::default();
    if args.bot {
        buf_flags |= BufferFlags::BEGINNING_OF_TEXT;
    }
    if args.eot {
        buf_flags |= BufferFlags::END_OF_TEXT;
    }
    if args.preserve_default_ignorables {
        buf_flags |= BufferFlags::PRESERVE_DEFAULT_IGNORABLES;
    }
    if args.remove_default_ignorables {
        buf_flags |= BufferFlags::REMOVE_DEFAULT_IGNORABLES;
    }
    if args.unsafe_to_concat {
        buf_flags |= BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
    }
    if args.safe_to_insert_tatweel {
        buf_flags |= BufferFlags::PRODUCE_SAFE_TO_INSERT_TATWEEL;
    }
    if args.verify {
        buf_flags |= BufferFlags::VERIFY;
    }

    let no_clusters = args.no_clusters || args.ned;
    let format_flags = {
        let mut f = SerializeFlags::default();
        if args.no_glyph_names {
            f |= SerializeFlags::NO_GLYPH_NAMES;
        }
        if no_clusters {
            f |= SerializeFlags::NO_CLUSTERS;
        }
        if args.no_positions {
            f |= SerializeFlags::NO_POSITIONS;
        }
        if args.no_advances || args.ned {
            f |= SerializeFlags::NO_ADVANCES;
        }
        if args.show_extents {
            f |= SerializeFlags::GLYPH_EXTENTS;
        }
        if args.show_flags {
            f |= SerializeFlags::GLYPH_FLAGS;
        }
        f.bits()
    };

    let language = args.language;
    let features = &args.features;

    let text = if let Some(ref path) = args.text_file {
        if path == &PathBuf::from("-") {
            read_stdin()?
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| format!("Error: cannot read '{}': {e}", path.display()))?
        }
    } else if font_set_as_free_arg {
        if let Some(ref text) = args.text_pos {
            text.clone()
        } else if let Some(ref text) = args.unicodes {
            text.clone()
        } else if let Some(ref text) = args.text {
            text.clone()
        } else {
            read_stdin()?
        }
    } else if let Some(ref text) = args.font_file_pos {
        text.to_string_lossy().to_string()
    } else if let Some(ref text) = args.unicodes {
        text.clone()
    } else if let Some(ref text) = args.text {
        text.clone()
    } else {
        read_stdin()?
    };

    let lines: Vec<&str> = if args.single_par {
        vec![&text]
    } else {
        text.split('\n').filter(|s| !s.is_empty()).collect()
    };

    let mut output = Vec::new();
    for (line_idx, text) in lines.iter().enumerate() {
        let line_no = line_idx + 1;

        if args.show_text {
            if args.show_line_num {
                write!(output, "{line_no}: ").unwrap();
            }
            writeln!(output, "({text})").unwrap();
        }

        if args.show_unicode {
            if args.show_line_num {
                write!(output, "{line_no}: ").unwrap();
            }
            writeln!(output, "{}", serialize_unicode(text, args.utf8_clusters)).unwrap();
        }

        let glyph_buffer = {
            let mut result = None;
            for _ in 0..args.num_iterations {
                let mut buffer = UnicodeBuffer::new();
                buffer.push_str(text);

                if let Some(d) = args.direction {
                    buffer.set_direction(d);
                }
                if let Some(ref lang) = language {
                    buffer.set_language(lang.clone());
                }
                if let Some(script) = args.script {
                    buffer.set_script(script);
                }

                buffer.set_cluster_level(args.cluster_level);
                if !args.utf8_clusters {
                    buffer.reset_clusters();
                }
                if let Some(g) = args.not_found_variation_selector_glyph {
                    buffer.set_not_found_variation_selector_glyph(g);
                }

                buffer.set_flags(buf_flags);

                if let Some(ctx) = pre_context {
                    buffer.set_pre_context(ctx);
                }
                if let Some(ctx) = post_context {
                    buffer.set_post_context(ctx);
                }

                buffer.guess_segment_properties();

                result = Some(shaper.shape(buffer, features));
            }
            result.unwrap()
        };

        if args.show_line_num {
            write!(output, "{line_no}: ").unwrap();
        }
        writeln!(
            output,
            "{}",
            glyph_buffer.serialize(&shaper, SerializeFlags::from_bits_truncate(format_flags))
        )
        .unwrap();
    }

    String::from_utf8(output).map_err(|e| format!("Error: invalid UTF-8 output: {e}"))
}

fn normalize_args(args: &mut Args) {
    if args.short_v {
        args.verbose = true;
        args.ned = true;
    }
    if args.verbose {
        args.show_text = true;
        args.show_unicode = true;
        args.show_line_num = true;
    }
}

fn write_output(output: &str, output_file: Option<&PathBuf>) -> Result<(), String> {
    if let Some(path) = output_file {
        let mut file = std::fs::File::create(path)
            .map_err(|e| format!("Error: cannot create '{}': {e}", path.display()))?;
        file.write_all(output.as_bytes())
            .map_err(|e| format!("Error: cannot write '{}': {e}", path.display()))?;
    } else {
        io::stdout()
            .lock()
            .write_all(output.as_bytes())
            .map_err(|e| format!("Error: writing stdout: {e}"))?;
    }

    Ok(())
}

fn read_stdin() -> Result<String, String> {
    io::read_to_string(io::stdin()).map_err(|e| format!("Error: reading stdin: {e}"))
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

fn parse_cluster(s: &str) -> Result<BufferClusterLevel, String> {
    match s {
        "0" => Ok(BufferClusterLevel::MonotoneGraphemes),
        "1" => Ok(BufferClusterLevel::MonotoneCharacters),
        "2" => Ok(BufferClusterLevel::Characters),
        "3" => Ok(BufferClusterLevel::Graphemes),
        _ => Err("invalid cluster level".to_string()),
    }
}

fn serialize_unicode(text: &str, utf8_clusters: bool) -> String {
    use std::fmt::Write;

    let mut s = String::new();
    let mut byte_offset = 0;
    for (char_idx, c) in text.chars().enumerate() {
        s.push(if s.is_empty() { '<' } else { '|' });
        let cluster = if utf8_clusters { byte_offset } else { char_idx };
        write!(s, "U+{:04X}={cluster}", c as u32).unwrap();
        byte_offset += c.len_utf8();
    }
    if !s.is_empty() {
        s.push('>');
    }
    s
}
