use std::{fs, path::PathBuf};

use harfrust::{font::Font, shape, Buffer, ShapeOptions, ShaperFont};

fn font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fonts")
        .join("text-rendering-tests")
        .join(name)
}

#[test]
fn hangul_tone_reordering_preserves_monotone_character_clusters() {
    assert_eq!(
        crate::shape(
            "tests/fonts/text-rendering-tests/FDArrayTest257.otf",
            "\u{AC00}\u{302E}",
            "--cluster-level=1 --show-flags --no-glyph-names",
        ),
        "[47=0+1000|1=0+1000]"
    );
}

// This test uses `TestGPOSThree.ttf`, which has a GPOS table.
// It verifies that GPOS attachment offset propagation scales safely without integer overflow
// for extremely long grapheme clusters (as fixed in commit 6bccd74).
#[test]
fn issue_384_overly_long_grapheme_cluster_gpos_does_not_overflow() {
    let font_data = fs::read(font_path("TestGPOSThree.ttf")).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    let shaping_font = shaper;

    let mut text = String::with_capacity(35_002);
    text.push('e');
    text.extend(std::iter::repeat_n('\u{0301}', 35_000));
    text.push('X');

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
}

// Uses `Calculator-Regular.ttf` (no GPOS table) to verify fallback mark positioning.
// A large scale factor amplifies mark heights, triggering fallback integer overflows
// with a lower grapheme count of 5,000.
#[test]
fn issue_384_overly_long_grapheme_cluster_fallback_does_not_overflow() {
    let font_data =
        fs::read(font_path("Calculator-Regular.ttf")).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    let mut shaping_font = shaper;

    let mut text = String::with_capacity(5002);
    text.push('e');
    text.extend(std::iter::repeat_n('\u{0301}', 5000));
    text.push('X');

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shaping_font.set_scale(10_000_000);
    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
}

// The glyph is 2143 units tall in a 2048-unit line. Its synthesized origin
// halves -95 toward negative infinity, matching HarfBuzz.
#[test]
fn synthesized_vertical_origin_floors_negative_half_height() {
    assert_eq!(
        crate::shape(
            "tests/fonts/text-rendering-tests/Calculator-Regular.ttf",
            "\u{03AE}",
            "--direction=ttb --no-glyph-names",
        ),
        "[825=0@-628,-1603+0,-2048]"
    );
}

#[test]
fn shaping_long_line_kern_does_not_overflow_glyph_data() {
    let font_data = fs::read(font_path("TestKERNOne.otf")).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);
    let shaping_font = shaper;

    let mut text = String::with_capacity(70_000);
    for _ in 0..35_000 {
        text.push('u');
        text.push('T');
    }

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
}

/// A mark more than `i16::MAX` glyphs after its base cannot store that
/// distance in its attachment chain. HarfBuzz leaves such a mark unattached
/// instead of storing a truncated chain.
#[test]
fn mark_too_far_from_its_base_is_left_unattached() {
    let mut text = String::from("x");
    text.extend(std::iter::repeat_n('\u{0301}', 32_769));
    let output = crate::shape(
        "tests/fonts/rb_custom/BungeeTint-Regular.ttf",
        &text,
        "--features=-mkmk --no-glyph-names --no-clusters",
    );
    let glyphs: Vec<&str> = output.trim_matches(['[', ']']).split('|').collect();
    assert_eq!(glyphs.len(), 32_770);
    assert_eq!(glyphs[32_768], "975@-369,0+0");
    assert_eq!(glyphs[32_769], "975+0");
}

// Sorting these marks temporarily puts the minimum cluster inside the run.
// It must remain exempt from flags applied to the other clusters.
#[test]
fn reordered_character_clusters_keep_the_minimum_boundary_safe() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/BungeeTint-Regular.ttf",
            "\u{030B}\u{0323}\u{031B}",
            "--cluster-level=2 --show-flags --no-glyph-names",
        ),
        "[986=2+0#1|987=1+0#1|984=0+0]"
    );
}

/// A zero-valued PairPosFormat1 record is still a concat hazard. In the test
/// font, changing `X` to `V` selects another record and changes `A`'s advance.
#[test]
fn pair_pos_format1_zero_record_is_unsafe_to_concat() {
    assert_eq!(
        crate::shape(
            "tests/fonts/pairpos1-zero-record.otf",
            "BAXB",
            "--features=test --no-glyph-names --no-positions --no-clusters \
             --show-flags --unsafe-to-concat",
        ),
        "[66|65#2|88#2|66]"
    );
}

/// A ligature set that cannot possibly match reports an unsafe-to-concat
/// hazard, whichever path reaches that conclusion.
///
/// `LigatureSet::apply` has two of them: a walk over the ligatures, and a fast
/// path that first asks a digest whether any ligature takes this second glyph
/// at all. The walk reports a hazard when a multi-component ligature wants a
/// different second glyph; the digest was returning without reporting one, so
/// the same font and the same text gave different flags depending on which
/// path ran. These two fonts are where that showed.
///
/// HarfBuzz 14.1.0 -- and its main branch, which has not touched these files
/// since -- reports no hazard here. Its own two paths disagree the same way,
/// and the walk is the one that looks right, so this follows the walk.
#[test]
fn ligature_digest_reports_the_same_concat_hazard_as_the_walk() {
    for font in [
        "tests/fonts/aots/gsub4_1_multiple_ligatures_f1.otf",
        "tests/fonts/aots/gsub4_1_multiple_ligatures_f2.otf",
    ] {
        assert_eq!(
            crate::shape(
                font,
                "\u{0012}\u{0015}",
                "--features=test --no-clusters --no-glyph-names --ned \
                 --show-flags --unsafe-to-concat",
            ),
            "[18#2|21@1500,0#2]",
            "{font}"
        );
    }
}

// Nikhahit moves ahead of every above-base mark before SARA AM, keeping each
// mark rather than repeating the first.
#[test]
fn thai_sara_am_after_two_above_base_marks() {
    assert_eq!(
        crate::shape(
            "tests/fonts/in-house/63a539a90a371ccf028dc2dcced9b63b07163be7.ttf",
            "\u{0E01}\u{0E48}\u{0E49}\u{0E33}",
            "",
        ),
        "[uni0E01=0+500|uni0E4D=0+0|uni0E48=0+0|uni0E49=0+0|uni0E32=0+500]"
    );
}

#[test]
fn tatweel_flag_production_cleans_singleton_cluster_flags() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/BungeeTint-Regular.ttf",
            "\u{0302}\u{0300}",
            "--cluster-level=2 --safe-to-insert-tatweel --show-flags --no-glyph-names",
        ),
        "[976=0+0|974=1@0,249+0#1]"
    );
}

#[test]
fn singleton_tatweel_opportunity_is_unsafe_to_break() {
    assert_eq!(
        crate::shape(
            "tests/fonts/in-house/73c3222a2992bac9067663888d2a1503774976bb.ttf",
            "\u{0628}\u{0628}",
            "--safe-to-insert-tatweel --show-flags --no-glyph-names",
        ),
        "[5=1+200#5|1=0+200]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_list_end() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-list-end.ttf", "aab", ""),
        "[a=0@50,0+611|a=1@40,0+601|b=2@31,0+646]"
    );
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-list-end.ttf",
            "aab",
            "--show-flags --unsafe-to-concat"
        ),
        "[a=0@50,0+611#2|a=1@40,0+601#3|b=2@31,0+646#3]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_negative_list() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-negative-list.ttf", "aab", ""),
        "[a=0@-50,0+511|a=1@-40,0+521|b=2@-31,0+584]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_value_index() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-value-index.ttf", "ab", ""),
        "[a=0+561|b=1@30,0+645]"
    );
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-value-index.ttf", "cb", ""),
        "[c=0+480|b=1@70,0+685]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_short_list() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-short-list.ttf", "aab", ""),
        "[a=0+561|a=1+561|b=2@31,0+646]"
    );
}

// https://github.com/harfbuzz/harfbuzz/issues/6536
#[test]
fn kerx_format1_retained_stack() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-retained-stack.ttf", "aab", ""),
        "[a=0@17,0+578|a=1@13,0+574|b=2@31,0+646]"
    );
}

// The format 4 subtable runs backwards, attaching each fatha to its base.
// Disable format 1 kerning so this regression isolates attachment direction.
#[test]
fn kerx_process_direction_arabic_marks() {
    assert_eq!(
        crate::shape(
            "tests/fonts/rb_custom/kerx-backwards-arabic.ttf",
            "\u{0648}\u{064E}\u{0644}\u{064E}\u{062F}",
            "--features=-kern",
        ),
        "[uni062F.fina=4+474|uni064E=2@-10,-28+0|uni0644.init=2+212|uni064E=0@157,-211+0|uni0648=0+468]"
    );
}

#[test]
fn kerx_process_direction_ltr_marks() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-backwards-mark.ttf", "xb", ""),
        "[m=0@325,600+150|b=1+615]"
    );
}

#[test]
fn kerx_process_direction_ltr_kerning() {
    assert_eq!(
        crate::shape("tests/fonts/rb_custom/kerx-backwards-kern.ttf", "ba", ""),
        "[b=0@30,0+645|a=1+561]"
    );
}

/// A lookup that covers a default-ignorable glyph starts on every glyph of an
/// ignorable run, and each start scans to the end of the run because the
/// skipping iterator steps over those same ignorables. That is quadratic in
/// the run, and the skipped glyphs must be charged to the shaping budget.
///
/// The test font maps `A` to glyph 1 and both `B` and U+200B to glyph 2, and
/// lists one PairPos lookup (glyph 2 before glyph 1) 32 times under `kern`.
/// With `B` the scans are one step each and the text shapes within budget;
/// with U+200B the same glyph string needs 16 million steps, so the work
/// limit has to trip.
#[test]
fn rescanning_default_ignorables_is_charged_to_the_shaping_budget() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/pairpos1-ignorable-rescan.ttf");
    let font_data = fs::read(path).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaping_font = ShaperFont::new(&instance);

    for (filler, within_budget) in [('B', true), ('\u{200B}', false)] {
        let mut text = String::from("A");
        text.extend(std::iter::repeat_n(filler, 999));

        let mut buffer = Buffer::new();
        buffer.push_str(&text);
        buffer.guess_segment_properties();

        shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();
        assert_eq!(buffer.allocation_successful(), within_budget);
    }
}

/// A match starting at a mark of an earlier ligature walks the output buffer
/// back over every mark of that ligature to find the ligature itself. The
/// font ligates `a b` over the marks, so all 6000 `c` belong to the ligature,
/// and then tries `c d g` at the last `c` 100 x 100 times through nested
/// lookups. Each try walks the 6000 marks again; charged to the operation
/// budget, that ends the text after a few thousand tries, where it would
/// otherwise cost the budget times the text length.
#[test]
fn ligature_base_walk_is_charged_to_the_operation_budget() {
    let font_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fonts/ligature-mark-walk.ttf");
    let font_data = fs::read(font_path).expect("failed to read test font");
    let font = Font::new(font_data, 0).expect("failed to parse test font");
    let instance = font.instance_builder().build();
    let shaper = ShaperFont::new(&instance);

    let mut text = String::with_capacity(6003);
    text.push('a');
    text.extend(std::iter::repeat_n('c', 6000));
    text.push_str("bd");

    let mut buffer = Buffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();

    shape(&shaper, &mut buffer, ShapeOptions::new()).unwrap();
    assert!(!buffer.allocation_successful());
}

/// A GSUB table whose `liga` feature expands glyph 0 into 255 copies of
/// itself twice, then lists `aliased` lookups that all point at one lookup
/// of `subtables` SingleSubst subtables covering glyphs 1 and 64 in turn.
/// The union of those digests admits glyph 0, so the lookup is walked at
/// every glyph of the buffer although no subtable can apply.
fn gsub_with_no_op_lookups(aliased: u16, subtables: u16) -> Vec<u8> {
    fn be16(bytes: &mut Vec<u8>, values: impl IntoIterator<Item = u16>) {
        for value in values {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
    }

    // SingleSubstFormat1 with a one-glyph coverage at 6 and delta 1.
    fn single_subst_covering(glyph: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        be16(&mut bytes, [1, 6, 1, 1, 1, glyph]);
        bytes
    }

    let covers_1 = single_subst_covering(1);
    let covers_64 = single_subst_covering(64);
    let subtable_size = covers_1.len() as u16;
    let header_size = 6 + 2 * subtables;
    let mut no_op_lookup = Vec::new();
    be16(&mut no_op_lookup, [1, 0, subtables]);
    be16(
        &mut no_op_lookup,
        (0..subtables).map(|i| header_size + (i % 2) * subtable_size),
    );
    no_op_lookup.extend_from_slice(&covers_1);
    no_op_lookup.extend_from_slice(&covers_64);

    // MultipleSubstFormat1 with a one-glyph coverage at 8 and one sequence
    // at 14, as the only subtable of a lookup.
    let mut expand_lookup = Vec::new();
    be16(&mut expand_lookup, [2, 0, 1, 8, 1, 8, 1, 14, 1, 1, 0, 255]);
    be16(&mut expand_lookup, std::iter::repeat_n(0, 255));

    // Lookups 0 and 1 expand; the rest alias the no-op lookup.
    let lookup_count = 2 + aliased;
    let no_op_offset = 2 + 2 * lookup_count;
    let expand_offset = no_op_offset + no_op_lookup.len() as u16;
    let mut lookup_list = Vec::new();
    be16(
        &mut lookup_list,
        [lookup_count, expand_offset, expand_offset],
    );
    be16(
        &mut lookup_list,
        std::iter::repeat_n(no_op_offset, usize::from(aliased)),
    );
    lookup_list.extend_from_slice(&no_op_lookup);
    lookup_list.extend_from_slice(&expand_lookup);

    let mut feature_list = Vec::new();
    be16(&mut feature_list, [1]);
    feature_list.extend_from_slice(b"liga");
    be16(&mut feature_list, [8, 0, lookup_count]);
    be16(&mut feature_list, 0..lookup_count);

    let mut script_list = Vec::new();
    be16(&mut script_list, [1]);
    script_list.extend_from_slice(b"DFLT");
    be16(&mut script_list, [8, 4, 0, 0, 0xFFFF, 1, 0]);

    let feature_list_offset = 10 + script_list.len() as u16;
    let lookup_list_offset = feature_list_offset + feature_list.len() as u16;
    let mut gsub = Vec::new();
    be16(
        &mut gsub,
        [1, 0, 10, feature_list_offset, lookup_list_offset],
    );
    gsub.extend_from_slice(&script_list);
    gsub.extend_from_slice(&feature_list);
    gsub.extend_from_slice(&lookup_list);
    gsub
}

/// Shapes "a", which maps to glyph 0, with a font of `gsub` alone.
fn shape_with_gsub(gsub: Vec<u8>) -> Buffer {
    use harfrust::font::{Blob, TableFunction};
    use std::sync::Arc;

    let gsub = Blob::from(gsub);
    let table = TableFunction::new(Arc::new(move |tag| {
        (tag == harfrust::Tag::new(b"GSUB")).then(|| gsub.clone())
    }));
    let font = Font::new(table, 0).expect("failed to build test font");
    let shaper = ShaperFont::new(&font);

    let mut buffer = Buffer::new();
    buffer.push_str("a");
    buffer.guess_segment_properties();
    shape(&shaper, &mut buffer, ShapeOptions::new()).unwrap();
    buffer
}

/// A walk over a lookup's subtables that applies nothing is charged to the
/// operation limit, so a feature listing such a lookup many times cannot
/// make shaping take time that the limit never sees.
#[test]
fn lookup_walks_that_apply_nothing_are_charged_to_the_operation_limit() {
    // Two expansions of one character make 65025 glyphs under an operation
    // limit of 65536, and each no-op lookup is walked at all of them.
    let buffer = shape_with_gsub(gsub_with_no_op_lookups(8, 63));
    assert!(!buffer.allocation_successful());

    // Without the no-op lookups the expansions alone stay within the limit.
    let buffer = shape_with_gsub(gsub_with_no_op_lookups(0, 63));
    assert!(buffer.allocation_successful());
    assert_eq!(buffer.len(), 65025);
}

/// Builds a font from `tables`, which must be sorted by tag.
fn sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut data = 0x0001_0000u32.to_be_bytes().to_vec();
    data.extend((tables.len() as u16).to_be_bytes());
    data.extend([0; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, table) in tables {
        data.extend(*tag);
        data.extend(0u32.to_be_bytes());
        data.extend((offset as u32).to_be_bytes());
        data.extend((table.len() as u32).to_be_bytes());
        offset += table.len().next_multiple_of(4);
    }
    for (_, table) in tables {
        data.extend(table);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    data
}

fn be16(values: impl IntoIterator<Item = u16>) -> Vec<u8> {
    values.into_iter().flat_map(u16::to_be_bytes).collect()
}

/// A font mapping `a` to glyph 1 and `b` to glyph 2, with a `liga` lookup
/// holding one ChainedSequenceContextFormat1 subtable on glyph 1. Its rule
/// set claims `rule_count` rules, but every offset points at the same one:
/// `input_len` × `a`, a lookahead of `lookahead_len - 1` × `a` then `b`, and no
/// lookup records. On a run of `a` the lookahead matches until the buffer
/// ends and fails only there.
fn aliased_chain_rules_font(rule_count: u16, input_len: u16, lookahead_len: u16) -> Vec<u8> {
    let mut rule = be16([0, input_len]);
    rule.extend(be16(std::iter::repeat_n(1, usize::from(input_len) - 1)));
    rule.extend(be16([lookahead_len]));
    rule.extend(be16(std::iter::repeat_n(1, usize::from(lookahead_len) - 1)));
    rule.extend(be16([2, 0]));
    let rule_offset = 2 + 2 * rule_count;
    let mut rule_set = be16([rule_count]);
    rule_set.extend(be16(std::iter::repeat_n(
        rule_offset,
        usize::from(rule_count),
    )));
    rule_set.extend(rule);

    // Coverage of glyph 1 at offset 8 and the rule set at offset 14.
    let mut subtable = be16([1, 8, 1, 14, 1, 1, 1]);
    subtable.extend(rule_set);
    let mut lookup_list = be16([1, 4, 6, 0, 1, 8]);
    lookup_list.extend(subtable);
    let mut script_list = be16([1]);
    script_list.extend(b"DFLT");
    script_list.extend(be16([8, 4, 0, 0, 0xFFFF, 1, 0]));
    let mut feature_list = be16([1]);
    feature_list.extend(b"liga");
    feature_list.extend(be16([8, 0, 1, 0]));
    let mut gsub = be16([
        1,
        0,
        10,
        10 + script_list.len() as u16,
        10 + (script_list.len() + feature_list.len()) as u16,
    ]);
    gsub.extend(script_list);
    gsub.extend(feature_list);
    gsub.extend(lookup_list);

    // cmap format 12: a..b -> 1..2
    let mut cmap = be16([0, 1, 3, 10]);
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(be16([12, 0]));
    for value in [28u32, 0, 1, u32::from(b'a'), u32::from(b'b'), 1] {
        cmap.extend(value.to_be_bytes());
    }
    // 1000 units per em, one advance for the three glyphs.
    let mut head = vec![0; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
    let hmtx = be16([500, 0, 0, 0]);
    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    maxp.extend(3u16.to_be_bytes());
    sfnt(&[
        (b"GSUB", gsub),
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
    ])
}

/// Rule offsets may alias, so a rule set's count and its rules' lookahead
/// lengths make a walk cost far more than the font's size. The rules looked
/// at, even when they fail at once, and the glyphs they are matched against
/// are charged to the operation limit, which ends shaping the way a runaway
/// recursion does, instead of running for seconds per kilobyte of text.
#[test]
fn aliased_chain_rules_are_charged_to_the_operation_limit() {
    let text = "a".repeat(500);
    // A rule with two input glyphs and a lookahead of `b` alone fails on the
    // pre-match; a longer lookahead matches `a`s up to the end of the buffer.
    for (rule_count, input_len, lookahead_len) in
        [(1, 2, 1), (8000, 2, 1), (1, 1, 1000), (1024, 1, 1000)]
    {
        let font_data = aliased_chain_rules_font(rule_count, input_len, lookahead_len);
        let font = Font::new(font_data, 0).expect("failed to parse test font");
        let instance = font.instance_builder().build();
        let shaping_font = ShaperFont::new(&instance);

        let mut buffer = Buffer::new();
        buffer.push_str(&text);
        buffer.guess_segment_properties();
        shape(&shaping_font, &mut buffer, ShapeOptions::new()).unwrap();

        // The rules never match, so only the aliased ones exhaust the limit.
        assert_eq!(
            buffer.allocation_successful(),
            rule_count == 1,
            "{rule_count} rules, input {input_len}, lookahead {lookahead_len}"
        );
    }
}
