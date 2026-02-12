use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::str::FromStr;

use harfrust::{
    BufferClusterLevel, BufferFlags, Direction, Feature, FontRef, Language, SerializeFlags,
    ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};

const HELP: &str = "\
Usage: hr-shape [OPTIONS] <FONT-FILE> [TEXT]

Font options:
    --font-file PATH                        Set font file-name
    -y, --face-index INDEX                  Set face index [default: 0]
    --font-ptem NUMBER                      Set font point-size
    --variations LIST                       Comma-separated list of font variations
    --named-instance INDEX                  Set named-instance index [default: none]

Input options:
    --text TEXT                             Set input text
    --text-file PATH                        Set input text file-name (\"-\" for stdin)
    -u, --unicodes LIST                     Set input Unicode codepoints
                                            Examples: 'U+0056,U+0057'
    --text-before TEXT                      Set text context before each line
    --text-after TEXT                       Set text context after each line
    --unicodes-before LIST                  Set Unicode codepoints context before each line
    --unicodes-after LIST                   Set Unicode codepoints context after each line
    --single-par                            Treat text as single paragraph

Shaping options:
    --direction DIRECTION                   Set text direction (ltr/rtl/ttb/btt)
    --language LANG                         Set text language [default: $LANG]
    --script TAG                            Set text script as ISO-15924 tag
    --features LIST                         Comma-separated list of font features
    --utf8-clusters                         Use UTF-8 byte indices, not char indices
    --cluster-level N                       Cluster merging level [default: 0]
                                            [possible values: 0, 1, 2, 3]
    --bot                                   Treat text as beginning of paragraph
    --eot                                   Treat text as end of paragraph
    --preserve-default-ignorables           Preserve Default-Ignorable characters
    --remove-default-ignorables             Remove Default-Ignorable characters
    --not-found-variation-selector-glyph N  Glyph value to replace not-found
                                            variation-selector characters with
    --unsafe-to-concat                      Produce unsafe-to-concat glyph flag
    --safe-to-insert-tatweel                Produce safe-to-insert-tatweel glyph flag
    --verify                                Perform sanity checks on shaping results

Output syntax options:
    --show-text                             Prefix each line of output with its input text
    --show-unicode                          Prefix each line of output with its input codepoints
    --show-line-num                         Prefix each line of output with its line number
    -v, --verbose                           Prefix each line of output with all of the above
    --no-glyph-names                        Output glyph indices instead of names
    --no-positions                          Do not output glyph positions
    --no-advances                           Do not output glyph advances
    --no-clusters                           Do not output cluster indices
    --show-extents                          Output glyph extents
    --show-flags                            Output glyph flags
    --ned                                   No Extra Data; Do not output clusters or advances

Output options:
    -o, --output-file PATH                  Set output file-name [default: stdout]
    -n, --num-iterations N                  Run shaper N times [default: 1]

Other options:
    -h, --help                              Show help options
    --version                               Show version number
";

struct Args {
    help: bool,
    version: bool,
    // Font options
    font_file: Option<PathBuf>,
    face_index: u32,
    font_ptem: Option<f32>,
    variations: Vec<Variation>,
    named_instance: Option<usize>,
    // Input options
    text: Option<String>,
    text_file: Option<PathBuf>,
    unicodes: Option<String>,
    text_before: Option<String>,
    text_after: Option<String>,
    unicodes_before: Option<String>,
    unicodes_after: Option<String>,
    single_par: bool,
    // Shaping options
    direction: Option<Direction>,
    language: Language,
    script: Option<harfrust::Script>,
    features: Vec<Feature>,
    utf8_clusters: bool,
    cluster_level: BufferClusterLevel,
    bot: bool,
    eot: bool,
    preserve_default_ignorables: bool,
    remove_default_ignorables: bool,
    not_found_variation_selector_glyph: Option<u32>,
    unsafe_to_concat: bool,
    safe_to_insert_tatweel: bool,
    verify: bool,
    // Output syntax options
    show_text: bool,
    show_unicode: bool,
    show_line_num: bool,
    no_glyph_names: bool,
    no_positions: bool,
    no_advances: bool,
    no_clusters: bool,
    show_extents: bool,
    show_flags: bool,
    ned: bool,
    // Output options
    output_file: Option<PathBuf>,
    num_iterations: u32,
    // Positional
    free: Vec<String>,
}

fn parse_args() -> Result<Args, pico_args::Error> {
    let mut args = pico_args::Arguments::from_env();

    // -v maps to both --verbose and --ned (matching hb-shape behavior)
    let short_v = args.contains("-v");
    let long_verbose = args.contains("--verbose");
    let verbose = short_v || long_verbose;

    let mut parsed = Args {
        help: args.contains(["-h", "--help"]),
        version: args.contains("--version"),
        // Font options
        font_file: args.opt_value_from_str("--font-file")?,
        face_index: args
            .opt_value_from_str(["-y", "--face-index"])?
            .unwrap_or(0),
        font_ptem: args.opt_value_from_str("--font-ptem")?,
        variations: args
            .opt_value_from_fn("--variations", parse_variations)?
            .unwrap_or_default(),
        named_instance: args.opt_value_from_str("--named-instance")?,
        // Input options
        text: args.opt_value_from_str("--text")?,
        text_file: args.opt_value_from_str("--text-file")?,
        unicodes: args.opt_value_from_fn(["-u", "--unicodes"], parse_unicodes)?,
        text_before: args.opt_value_from_str("--text-before")?,
        text_after: args.opt_value_from_str("--text-after")?,
        unicodes_before: args.opt_value_from_fn("--unicodes-before", parse_unicodes)?,
        unicodes_after: args.opt_value_from_fn("--unicodes-after", parse_unicodes)?,
        single_par: args.contains("--single-par"),
        // Shaping options
        direction: args.opt_value_from_str("--direction")?,
        language: args
            .opt_value_from_str("--language")?
            .unwrap_or(system_language()),
        script: args.opt_value_from_str("--script")?,
        features: args
            .opt_value_from_fn("--features", parse_features)?
            .unwrap_or_default(),
        utf8_clusters: args.contains("--utf8-clusters"),
        cluster_level: args
            .opt_value_from_fn("--cluster-level", parse_cluster)?
            .unwrap_or_default(),
        bot: args.contains("--bot"),
        eot: args.contains("--eot"),
        preserve_default_ignorables: args.contains("--preserve-default-ignorables"),
        remove_default_ignorables: args.contains("--remove-default-ignorables"),
        not_found_variation_selector_glyph: args
            .opt_value_from_str("--not-found-variation-selector-glyph")?,
        unsafe_to_concat: args.contains("--unsafe-to-concat"),
        safe_to_insert_tatweel: args.contains("--safe-to-insert-tatweel"),
        verify: args.contains("--verify"),
        // Output syntax options
        show_text: args.contains("--show-text"),
        show_unicode: args.contains("--show-unicode"),
        show_line_num: args.contains("--show-line-num"),
        no_glyph_names: args.contains("--no-glyph-names"),
        no_positions: args.contains("--no-positions"),
        no_advances: args.contains("--no-advances"),
        no_clusters: args.contains("--no-clusters"),
        show_extents: args.contains("--show-extents"),
        show_flags: args.contains("--show-flags"),
        ned: args.contains("--ned"),
        // Output options
        output_file: args.opt_value_from_str(["-o", "--output-file"])?,
        num_iterations: args
            .opt_value_from_str(["-n", "--num-iterations"])?
            .unwrap_or(1),
        // Positional
        free: args
            .finish()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect(),
    };

    if verbose {
        parsed.show_text = true;
        parsed.show_unicode = true;
        parsed.show_line_num = true;
    }
    // -v implies --ned (matching hb-shape where -v is shared by --verbose and --ned)
    if short_v {
        parsed.ned = true;
    }

    Ok(parsed)
}

fn main() {
    let args = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {e}.");
            std::process::exit(1);
        }
    };

    if args.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.help {
        print!("{HELP}");
        return;
    }

    // Resolve font path from --font-file or first positional arg
    let mut font_set_as_free_arg = false;
    let font_path = if let Some(path) = args.font_file.clone() {
        path
    } else if !args.free.is_empty() {
        font_set_as_free_arg = true;
        PathBuf::from(&args.free[0])
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
    let instance = match args.named_instance {
        Some(idx) => {
            let mut inst = ShaperInstance::from_named_instance(&font, idx);
            if !args.variations.is_empty() {
                inst.set_variations(&font, &args.variations);
            }
            inst
        }
        None => ShaperInstance::from_variations(&font, &args.variations),
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
    } else if args.free.len() == 2 && font_set_as_free_arg {
        args.free[1].clone()
    } else if args.free.len() == 1 && !font_set_as_free_arg {
        args.free[0].clone()
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
                buffer.set_language(args.language.clone());
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

                result = Some(shaper.shape(buffer, &args.features));
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
    let mut text = String::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap_or_else(|e| {
            eprintln!("Error: reading stdin: {e}");
            std::process::exit(1);
        });
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
    }
    text
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

fn parse_features(s: &str) -> Result<Vec<Feature>, String> {
    let mut features = Vec::new();
    for f in s.split(',') {
        features.push(Feature::from_str(f)?);
    }
    Ok(features)
}

fn parse_variations(s: &str) -> Result<Vec<Variation>, String> {
    let mut variations = Vec::new();
    for v in s.split(',') {
        variations.push(Variation::from_str(v)?);
    }
    Ok(variations)
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
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
        let s = libc::setlocale(libc::LC_CTYPE, std::ptr::null());
        let s = std::ffi::CStr::from_ptr(s);
        let s = s.to_str().expect("locale must be ASCII");
        Language::from_str(s).unwrap()
    }
}
