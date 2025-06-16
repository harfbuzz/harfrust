mod aots;
mod custom;
mod in_house;
mod macos;
mod text_rendering_tests;

use harfrust::{BufferFlags, FontRef, ShaperData, ShaperInstance};
use std::str::FromStr;

struct Args {
    face_index: u32,
    font_ptem: Option<f32>,
    variations: Vec<String>,
    direction: Option<harfrust::Direction>,
    language: Option<harfrust::Language>,
    script: Option<harfrust::Script>,
    #[allow(dead_code)]
    remove_default_ignorables: bool,
    unsafe_to_concat: bool,
    not_found_variation_selector_glyph: Option<u32>,
    cluster_level: harfrust::BufferClusterLevel,
    features: Vec<String>,
    pre_context: Option<String>,
    post_context: Option<String>,
    no_glyph_names: bool,
    no_positions: bool,
    no_advances: bool,
    no_clusters: bool,
    show_extents: bool,
    show_flags: bool,
    ned: bool,
    bot: bool,
    eot: bool,
}

fn parse_args(args: Vec<std::ffi::OsString>) -> Result<Args, pico_args::Error> {
    let mut parser = pico_args::Arguments::from_vec(args);
    let args = Args {
        face_index: parser.opt_value_from_str("--face-index")?.unwrap_or(0),
        font_ptem: parser.opt_value_from_str("--font-ptem")?,
        variations: parser
            .opt_value_from_fn("--variations", parse_string_list)?
            .unwrap_or_default(),
        direction: parser.opt_value_from_str("--direction")?,
        language: parser.opt_value_from_str("--language")?,
        script: parser.opt_value_from_str("--script")?,
        remove_default_ignorables: parser.contains("--remove-default-ignorables"),
        unsafe_to_concat: parser.contains("--unsafe-to-concat"),
        not_found_variation_selector_glyph: parser
            .opt_value_from_str("--not-found-variation-selector-glyph")?,
        cluster_level: parser
            .opt_value_from_fn("--cluster-level", parse_cluster)?
            .unwrap_or_default(),
        features: parser
            .opt_value_from_fn("--features", parse_string_list)?
            .unwrap_or_default(),
        pre_context: parser
            .opt_value_from_fn("--unicodes-before", parse_unicodes)
            .unwrap_or_default(),
        post_context: parser
            .opt_value_from_fn("--unicodes-after", parse_unicodes)
            .unwrap_or_default(),
        no_glyph_names: parser.contains("--no-glyph-names"),
        no_positions: parser.contains("--no-positions"),
        no_advances: parser.contains("--no-advances"),
        no_clusters: parser.contains("--no-clusters"),
        show_extents: parser.contains("--show-extents"),
        show_flags: parser.contains("--show-flags"),
        ned: parser.contains("--ned"),
        bot: parser.contains("--bot"),
        eot: parser.contains("--eot"),
    };

    Ok(args)
}

fn parse_string_list(s: &str) -> Result<Vec<String>, String> {
    Ok(s.split(',').map(|s| s.to_string()).collect())
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
    let args = options
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(std::ffi::OsString::from)
        .collect();
    let args = parse_args(args).unwrap();

    let font_data =
        std::fs::read(font_path).unwrap_or_else(|e| panic!("Could not read {}: {}", font_path, e));
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
    if let Some(pre_context) = args.pre_context {
        buffer.set_pre_context(&pre_context);
    }
    buffer.push_str(text);
    if let Some(post_context) = args.post_context {
        buffer.set_post_context(&post_context);
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

    let mut features = Vec::new();
    for feature_str in args.features {
        let feature = harfrust::Feature::from_str(&feature_str).unwrap();
        features.push(feature);
    }

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
