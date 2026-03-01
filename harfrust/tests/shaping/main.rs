mod aots;
mod custom;
mod in_house;
mod macos;
mod text_rendering_tests;

use clap::Parser;
use harfrust::{BufferFlags, FontRef, ShaperData, ShaperInstance};
use std::str::FromStr;

#[derive(Parser)]
#[command(no_binary_name = true)]
struct Args {
    #[arg(long, default_value_t = 0)]
    face_index: u32,

    #[arg(long)]
    font_ptem: Option<f32>,

    #[arg(long, value_delimiter = ',')]
    variations: Vec<String>,

    #[arg(long)]
    direction: Option<harfrust::Direction>,

    #[arg(long)]
    language: Option<harfrust::Language>,

    #[arg(long)]
    script: Option<harfrust::Script>,

    #[allow(dead_code)]
    #[arg(long)]
    remove_default_ignorables: bool,

    #[arg(long)]
    unsafe_to_concat: bool,

    #[arg(long)]
    not_found_variation_selector_glyph: Option<u32>,

    #[arg(long, value_parser = parse_cluster, default_value = "0")]
    cluster_level: harfrust::BufferClusterLevel,

    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,

    #[arg(long, value_parser = parse_unicodes)]
    unicodes_before: Option<String>,

    #[arg(long, value_parser = parse_unicodes)]
    unicodes_after: Option<String>,

    #[arg(long)]
    no_glyph_names: bool,

    #[arg(long)]
    no_positions: bool,

    #[arg(long)]
    no_advances: bool,

    #[arg(long)]
    no_clusters: bool,

    #[arg(long)]
    show_extents: bool,

    #[arg(long)]
    show_flags: bool,

    #[arg(long)]
    ned: bool,

    #[arg(long)]
    bot: bool,

    #[arg(long)]
    eot: bool,

    /// Ignored (hb-shape compat)
    #[arg(long, hide = true)]
    font_funcs: Option<String>,
}

fn parse_cluster(s: &str) -> Result<harfrust::BufferClusterLevel, String> {
    match s {
        "0" => Ok(harfrust::BufferClusterLevel::MonotoneGraphemes),
        "1" => Ok(harfrust::BufferClusterLevel::MonotoneCharacters),
        "2" => Ok(harfrust::BufferClusterLevel::Characters),
        "3" => Ok(harfrust::BufferClusterLevel::Graphemes),
        _ => Err("invalid cluster level".to_string()),
    }
}

fn parse_unicodes(s: &str) -> Result<String, String> {
    s.split(',')
        .map(|s| {
            let s = s.strip_prefix("U+").unwrap_or(s);
            let cp = u32::from_str_radix(s, 16).map_err(|e| format!("{e}"))?;
            char::from_u32(cp).ok_or_else(|| format!("{cp:X} is not a valid codepoint"))
        })
        .collect()
}

pub fn shape(font_path: &str, text: &str, options: &str) -> String {
    // Strip shell-style quotes that test strings use around values
    let option_args: Vec<String> = options
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(|s| s.replace('"', ""))
        .collect();
    let args = Args::try_parse_from(&option_args).unwrap();

    let font_data =
        std::fs::read(font_path).unwrap_or_else(|e| panic!("Could not read {font_path}: {e}"));
    let font = FontRef::from_index(&font_data, args.face_index).unwrap();

    let variations: Vec<_> = args
        .variations
        .iter()
        .map(|s| harfrust::Variation::from_str(s).unwrap())
        .collect();

    let data = ShaperData::new(&font);
    let instance =
        (!variations.is_empty()).then(|| ShaperInstance::from_variations(&font, &variations));
    let shaper = data
        .shaper(&font)
        .instance(instance.as_ref())
        .point_size(args.font_ptem)
        .build();

    let mut buffer = harfrust::UnicodeBuffer::new();
    if let Some(ref pre_context) = args.unicodes_before {
        buffer.set_pre_context(pre_context);
    }
    buffer.push_str(text);
    if let Some(ref post_context) = args.unicodes_after {
        buffer.set_post_context(post_context);
    }

    if let Some(d) = args.direction {
        buffer.set_direction(d);
    }

    if let Some(g) = args.not_found_variation_selector_glyph {
        buffer.set_not_found_variation_selector_glyph(g);
    }

    if let Some(lang) = args.language {
        buffer.set_language(lang);
    }

    if let Some(script) = args.script {
        buffer.set_script(script);
    }

    let mut buffer_flags = BufferFlags::default();
    buffer_flags.set(BufferFlags::BEGINNING_OF_TEXT, args.bot);
    buffer_flags.set(BufferFlags::END_OF_TEXT, args.eot);
    buffer_flags.set(BufferFlags::PRODUCE_UNSAFE_TO_CONCAT, args.unsafe_to_concat);
    buffer_flags.set(
        BufferFlags::REMOVE_DEFAULT_IGNORABLES,
        args.remove_default_ignorables,
    );
    buffer.set_flags(buffer_flags);

    buffer.set_cluster_level(args.cluster_level);
    buffer.reset_clusters();

    let features: Vec<_> = args
        .features
        .iter()
        .map(|s| harfrust::Feature::from_str(s).unwrap())
        .collect();

    buffer.guess_segment_properties();
    let glyph_buffer = shaper.shape(buffer, &features);

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

    glyph_buffer.serialize(&shaper, format_flags)
}
