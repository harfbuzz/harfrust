use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use harfrust::{FontRef, ShaperData, ShaperInstance};

#[derive(Parser)]
#[command(name = "shape", version, about = "Shape text using HarfRust")]
struct Args {
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
    #[arg(long, default_value_t = 0)]
    face_index: u32,

    /// Set font point-size
    #[arg(long)]
    font_ptem: Option<f32>,

    /// Comma-separated list of font variations
    #[arg(long, value_parser = parse_variations)]
    variations: Option<Vec<harfrust::Variation>>,

    /// Set input text
    #[arg(long)]
    text: Option<String>,

    /// Set input text file
    #[arg(long)]
    text_file: Option<PathBuf>,

    /// Set comma-separated list of input Unicode codepoints (e.g. 'U+0056,U+0057')
    #[arg(short = 'u', long, value_parser = parse_unicodes)]
    unicodes: Option<String>,

    /// Set text direction (ltr/rtl/ttb/btt)
    #[arg(long)]
    direction: Option<harfrust::Direction>,

    /// Set text language [default: $LANG]
    #[arg(long)]
    language: Option<harfrust::Language>,

    /// Set text script as ISO-15924 tag
    #[arg(long)]
    script: Option<harfrust::Script>,

    /// Glyph value to replace not-found variation-selector characters with
    #[arg(long)]
    not_found_variation_selector_glyph: Option<u32>,

    /// Use UTF-8 byte indices, not char indices
    #[arg(long)]
    utf8_clusters: bool,

    /// Cluster merging level (0-2)
    #[arg(long, value_parser = parse_cluster, default_value = "0")]
    cluster_level: harfrust::BufferClusterLevel,

    /// Comma-separated list of font features
    #[arg(long, value_parser = parse_features)]
    features: Option<Vec<harfrust::Feature>>,

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

    /// Treat the input string as a single paragraph
    #[arg(long)]
    single_par: bool,

    /// No Extra Data; Do not output clusters or advances
    #[arg(long)]
    ned: bool,
}

fn main() {
    let args = Args::parse();

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

    let font_data = std::fs::read(font_path).unwrap();
    let font = FontRef::from_index(&font_data, args.face_index).unwrap();
    let data = ShaperData::new(&font);
    let variations = args.variations.as_deref().unwrap_or_default();
    let instance = ShaperInstance::from_variations(&font, variations);
    let shaper = data
        .shaper(&font)
        .instance(Some(&instance))
        .point_size(args.font_ptem)
        .build();

    let language = args.language.unwrap_or_else(system_language);
    let features = args.features.as_deref().unwrap_or_default();

    let text = if let Some(ref path) = args.text_file {
        std::fs::read_to_string(path).unwrap()
    } else if font_set_as_free_arg {
        if let Some(ref text) = args.text_pos {
            text.clone()
        } else if let Some(ref text) = args.unicodes {
            text.clone()
        } else if let Some(ref text) = args.text {
            text.clone()
        } else {
            eprintln!("Error: text is not set.");
            std::process::exit(1);
        }
    } else if let Some(ref text) = args.font_file_pos {
        // font was set via --font-file, so first positional is text
        text.to_string_lossy().to_string()
    } else if let Some(ref text) = args.unicodes {
        text.clone()
    } else if let Some(ref text) = args.text {
        text.clone()
    } else {
        eprintln!("Error: text is not set.");
        std::process::exit(1);
    };

    let lines = if args.single_par {
        vec![text.as_str()]
    } else {
        text.split('\n').filter(|s| !s.is_empty()).collect()
    };

    for text in lines {
        let mut buffer = harfrust::UnicodeBuffer::new();
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

        buffer.guess_segment_properties();

        let glyph_buffer = shaper.shape(buffer, features);

        let mut format_flags = harfrust::SerializeFlags::default();
        if args.no_glyph_names {
            format_flags |= harfrust::SerializeFlags::NO_GLYPH_NAMES;
        }

        if args.no_clusters || args.ned {
            format_flags |= harfrust::SerializeFlags::NO_CLUSTERS;
        }

        if args.no_positions {
            format_flags |= harfrust::SerializeFlags::NO_POSITIONS;
        }

        if args.no_advances || args.ned {
            format_flags |= harfrust::SerializeFlags::NO_ADVANCES;
        }

        if args.show_extents {
            format_flags |= harfrust::SerializeFlags::GLYPH_EXTENTS;
        }

        if args.show_flags {
            format_flags |= harfrust::SerializeFlags::GLYPH_FLAGS;
        }

        println!("{}", glyph_buffer.serialize(&shaper, format_flags));
    }
}

fn parse_unicodes(s: &str) -> Result<String, String> {
    let mut text = String::new();
    for u in s.split(',') {
        let u = u32::from_str_radix(&u[2..], 16)
            .map_err(|_| format!("'{u}' is not a valid codepoint"))?;
        let c = char::try_from(u).map_err(|_| format!("{u} is not a valid codepoint"))?;
        text.push(c);
    }
    Ok(text)
}

fn parse_features(s: &str) -> Result<Vec<harfrust::Feature>, String> {
    let mut features = Vec::new();
    for f in s.split(',') {
        features.push(harfrust::Feature::from_str(f)?);
    }
    Ok(features)
}

fn parse_variations(s: &str) -> Result<Vec<harfrust::Variation>, String> {
    let mut variations = Vec::new();
    for v in s.split(',') {
        variations.push(harfrust::Variation::from_str(v)?);
    }
    Ok(variations)
}

fn parse_cluster(s: &str) -> Result<harfrust::BufferClusterLevel, String> {
    match s {
        "0" => Ok(harfrust::BufferClusterLevel::MonotoneGraphemes),
        "1" => Ok(harfrust::BufferClusterLevel::MonotoneCharacters),
        "2" => Ok(harfrust::BufferClusterLevel::Characters),
        _ => Err("invalid cluster level".to_string()),
    }
}

fn system_language() -> harfrust::Language {
    let locale = std::env::var("LC_CTYPE")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    harfrust::Language::from_str(&locale).unwrap()
}
