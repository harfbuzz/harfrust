//! Rust implementation of hb-shape.
//! <https://github.com/harfbuzz/harfbuzz/blob/main/util/hb-shape.cc>

use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use harfrust::{
    BufferClusterLevel, BufferFlags, Direction, Feature, FontRef, Language, SerializeFlags,
    ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};

#[derive(Parser)]
#[command(name = "hr-shape", version, about = "Shape text using HarfRust")]
struct Args {
    /// Font file path
    #[arg(value_name = "FONT-FILE")]
    font_file_pos: Option<PathBuf>,

    /// Text to shape
    #[arg(value_name = "TEXT")]
    text_pos: Option<String>,

    // Font options
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

    // Input options
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

    // Shaping options
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

    // Output syntax options
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

    // Output options
    /// Set output file-name [default: stdout]
    #[arg(short = 'o', long)]
    output_file: Option<PathBuf>,

    /// Run shaper N times
    #[arg(short = 'n', long, default_value_t = 1)]
    num_iterations: u32,
}

fn main() {
    let mut args = Args::parse();

    // -v implies --verbose --ned (matching hb-shape behavior)
    if args.short_v {
        args.verbose = true;
        args.ned = true;
    }
    if args.verbose {
        args.show_text = true;
        args.show_unicode = true;
        args.show_line_num = true;
    }

    // Resolve font path from --font-file or first positional arg
    let mut font_set_as_free_arg = false;
    let font_path = if let Some(ref path) = args.font_file {
        path.clone()
    } else if let Some(ref path) = args.font_file_pos {
        font_set_as_free_arg = true;
        path.clone()
    } else {
        eprintln!("Error: font is not set.");
        std::process::exit(1);
    };

    if !font_path.exists() {
        eprintln!("Error: '{}' does not exist.", font_path.display());
        std::process::exit(1);
    }

    let font_data = std::fs::read(&font_path).unwrap_or_else(|e| {
        eprintln!("Error: cannot read '{}': {e}", font_path.display());
        std::process::exit(1);
    });
    let font = FontRef::from_index(&font_data, args.face_index).unwrap_or_else(|_| {
        eprintln!("Error: face index {} not found.", args.face_index);
        std::process::exit(1);
    });

    // Build shaper
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

    // Resolve text context
    let pre_context = args
        .unicodes_before
        .as_deref()
        .or(args.text_before.as_deref());
    let post_context = args
        .unicodes_after
        .as_deref()
        .or(args.text_after.as_deref());

    // Resolve buffer flags
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

    // Resolve serialize flags
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

    let language = args.language.unwrap_or_else(system_language);
    let features = &args.features;

    // Resolve text input
    let text = if let Some(ref path) = args.text_file {
        if path == &PathBuf::from("-") {
            read_stdin()
        } else {
            std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("Error: cannot read '{}': {e}", path.display());
                std::process::exit(1);
            })
        }
    } else if font_set_as_free_arg {
        if let Some(ref text) = args.text_pos {
            text.clone()
        } else if let Some(ref text) = args.unicodes {
            text.clone()
        } else if let Some(ref text) = args.text {
            text.clone()
        } else {
            read_stdin()
        }
    } else if let Some(ref text) = args.font_file_pos {
        // font was set via --font-file, so first positional is text
        text.to_string_lossy().to_string()
    } else if let Some(ref text) = args.unicodes {
        text.clone()
    } else if let Some(ref text) = args.text {
        text.clone()
    } else {
        read_stdin()
    };

    let lines: Vec<&str> = if args.single_par {
        vec![&text]
    } else {
        text.split('\n').filter(|s| !s.is_empty()).collect()
    };

    // Open output
    let stdout = io::stdout();
    let mut out: Box<dyn Write> = if let Some(ref path) = args.output_file {
        Box::new(io::BufWriter::new(
            std::fs::File::create(path).unwrap_or_else(|e| {
                eprintln!("Error: cannot create '{}': {e}", path.display());
                std::process::exit(1);
            }),
        ))
    } else {
        Box::new(stdout.lock())
    };

    for (line_idx, text) in lines.iter().enumerate() {
        let line_no = line_idx + 1;

        // Show text prefix
        if args.show_text {
            if args.show_line_num {
                write!(out, "{line_no}: ").unwrap();
            }
            writeln!(out, "({text})").unwrap();
        }

        // Show unicode prefix
        if args.show_unicode {
            if args.show_line_num {
                write!(out, "{line_no}: ").unwrap();
            }
            writeln!(out, "{}", serialize_unicode(text, args.utf8_clusters)).unwrap();
        }

        // Shape (possibly multiple iterations for benchmarking)
        let glyph_buffer = {
            let mut result = None;
            for _ in 0..args.num_iterations {
                let mut buffer = UnicodeBuffer::new();
                buffer.push_str(text);

                if let Some(d) = args.direction {
                    buffer.set_direction(d);
                }
                buffer.set_language(language.clone());
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

        // Output glyphs
        if args.show_line_num {
            write!(out, "{line_no}: ").unwrap();
        }
        writeln!(
            out,
            "{}",
            glyph_buffer.serialize(&shaper, SerializeFlags::from_bits_truncate(format_flags))
        )
        .unwrap();
    }
}

fn read_stdin() -> String {
    io::read_to_string(io::stdin()).unwrap_or_else(|e| {
        eprintln!("Error: reading stdin: {e}");
        std::process::exit(1);
    })
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

fn system_language() -> Language {
    let locale = std::env::var("LC_CTYPE")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    Language::from_str(&locale).unwrap()
}
