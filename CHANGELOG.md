# Change Log

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

## [Unreleased]

This development version matches HarfBuzz [v14.3.1](https://github.com/harfbuzz/harfbuzz/releases/tag/14.3.1).

- Add a unified `Buffer` type, matching HarfBuzz's `hb_buffer_t`. It carries a
  `BufferContentType` describing whether it holds input characters or shaped
  glyphs, and is shaped in place with `Buffer::shape`. `UnicodeBuffer` and
  `GlyphBuffer` are unchanged, and convert to and from `Buffer`.

## [0.13.3] - 2026-08-25

This release matches HarfBuzz [v14.3.1](https://github.com/harfbuzz/harfbuzz/releases/tag/14.3.1),
and has an MSRV (minimum supported Rust version) of 1.85.

- Add support for shaping legacy AAT `mort` tables, including
  language-specific morph features (#451, #453).
- Handle tuple kerning in AAT `kerx` tables (#452).
- Fix a debug assertion for legacy `kern` state tables that declare more than
  256 classes (#455).
- Synthesize Arabic fallback GSUB lookups, including legacy Windows-1256
  glyph layouts and `no_std` builds (#454, #456).
- Update `read-fonts` to 0.43.3.

## [0.13.2] - 2026-08-23

This release matches HarfBuzz [v14.3.1](https://github.com/harfbuzz/harfbuzz/releases/tag/14.3.1),
and has an MSRV (minimum supported Rust version) of 1.85.

- Speed up contextual and chained-context lookup matching by probing raw rule
  offsets and reusing parsed coverage tables (#437).
- Skip the redundant per-subtable digest test for single-subtable lookups
  (#438).
- Set up glyph-set acceleration before applying legacy `kern` tables (#439).
- Precompute and pack AAT state-machine safe-to-break probes in the face cache
  (#445).
- Cache reusable shaping table metadata and avoid moving shaper and buffer
  temporaries on every shape (#446, #447).

## [0.13.1] - 2026-08-21

This release matches HarfBuzz [v14.3.1](https://github.com/harfbuzz/harfbuzz/releases/tag/14.3.1),
and has an MSRV (minimum supported Rust version) of 1.85.

- Saturate shaping and positioning math across GPOS, kerx, trak, fallback,
  shaping, metrics, and font scaling paths to avoid overflow and better match
  HarfBuzz behavior (#420).
- Cache GDEF mark filtering sets in bitmaps (#421).
- Lock `icu_properties` to 2.2.0 (#422).
- Optimize hot loops in contextual and layout matching by probing rules before
  parsing, hoisting invariants, moving recursion state instead of cloning, and
  replacing per-element copies with block copies (#423, #424, #425).
- Optimize buffer internals by copying small counts inline and charging
  `move_to` operations against `max_ops` (#426, #429).
- Improve AAT shaping performance by walking `morx` subtables through packed
  descriptors, resolving subtable kinds from cached parts, and adding a
  state-machine fast path (#428, #430, #431).
- Skip the post-`morx` digest update when GPOS is not present (#433).
- Add a batched nominal-glyph callback to `FontFuncs` (#434).
- Update `read-fonts` to 0.43.2 (#432, #435).

## [0.13.0] - 2026-08-12

This release matches HarfBuzz [v14.3.1](https://github.com/harfbuzz/harfbuzz/releases/tag/14.3.1),
and has an MSRV (minimum supported Rust version) of 1.85.

- Guard GPOS cursive attachment against out-of-bounds `attach_chain` offsets
  that could panic on buffers longer than 32767 glyphs (#411).
- Apply `calt` to non-Hangul characters in Hangul runs while continuing to
  suppress it for Jamo.
- Preserve the correct component assignments when MultipleSubst output is
  ligated and positioned with mark attachment lookups.
- Apply cross-direction mark attachment offsets immediately.
- Bound syllables to 64 glyphs during shaping.
- Map the collective ISO 15924 `Hrkt` script to the OpenType `kana` script tag.
- Update `read-fonts` to 0.43.0 (#418).

## [0.12.0] - 2026-07-03

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Update `read-fonts` to 0.41.0 (#403).
- Saturate AAT kerning position accumulation to avoid overflow (#400).
- Regenerate the Unicode data tables with the latest `packtab` (#401).
- Keep GPOS variation-index deltas fractional to match HarfBuzz, avoiding
  sub-font-unit advance and placement differences on variable fonts (#403).
- Keep AAT `trak` tracking fractional to match HarfBuzz (#404).

## [0.11.0] - 2026-06-29

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Parse contextual lookup rules in a single pass in the apply path for improved performance (#383).
- Fix integer overflow in fallback positioning shaper (#395).
- Fix overflow panic in skipping_iterator_t when shaping extremely long lines (#397).
- Fix custom nominal glyph cmap caching (#399).

## [0.10.0] - 2026-06-20

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Add support for the new `read-fonts` Font API behind the `experimental_font_api`
  feature flag in both `read-fonts` and `harfrust`, and update `read-fonts` to 0.40.2.
- Reject recursive extension lookups.
- Bound ligature cache construction.

## [0.9.0] - 2026-06-15

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Fix GPOS attachment offset overflow #384
- Require read-fonts 0.40.

## [0.8.4] - 2026-06-02

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Use bytes instead of str storage for language tag table.

## [0.8.3] - 2026-06-01

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Add method for appending bulk glyph infos to UnicodeBuffer.

## [0.8.2] - 2026-05-31

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Allow setting pre- and post-context from codepoints.
- Added `Language::new` constructor that accepts strings or bytes.
- Re-export `GlyphId` so that `FontFuncs` implementors don't need to add extra dependencies
  to name the type.
- Reify glyph flags into a `GlyphFlags` type. Existing methods on `GlyphInfo` remain for now
  to avoid a breaking change but will be removed in a future version.
- Fix AAT in place glyph deletion.

## [0.8.1] - 2026-05-28

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Added support for font function overrides to enable injection of hinted metrics and control
  over character mapping. See `FontFuncs` and `ShapeOptions::font_funcs`.
- Added internal scaling support. See `ShapeOptions::scale` and `ShapeOptions::scale_separate`.

## [0.8.0] - 2026-05-26

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Refactor ot shape context functions into methods.
- Added `ShapeOptions` type as argument to shape calls, preparation for being
  able to pass font functions. `ShapeOptions` accepts shape plan, point size, features.
  `shape_with_plan()` function removed in favor of new builder-style API.
- Document release process to avoid missing LICENSE files.

Release 0.7.1 was released prematurely and yanked from crates.io.

## [0.7.1] - 2026-05-26

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Document release process to avoid missing LICENSE files.

## [0.7.0] - 2026-05-21

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Make core_maths dependency optional and add new libm feature to control it. One of libm or std
  features is now required to build harfrust, matching the behavior of the fontations crates.
  This is a breaking change.

## [0.6.2] - 2026-05-21

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Do not hardcode requirement on libm for read-fonts dependency. Libm is required for read-fonts
  only in nostd environments.

## [0.6.1] - 2026-05-21

This release matches HarfBuzz [v14.2.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0),
and has an MSRV (minimum supported Rust version) of 1.85.

- Blocklist broken fonts in GDEF. This ports feature from HarfBuzz that was missing before.
- Add LICENSE file symlink to each crate.


## [0.6.0] - 2026-04-09

This release matches HarfBuzz [v14.1.0][harfbuzz-14.1.0], and has an MSRV (minimum supported Rust version) of 1.85.

Roll to `read-fonts` 0.39.0.

## [0.5.2] - 2026-03-04

This release matches HarfBuzz [v13.0.0][harfbuzz-13.0.0], and has an MSRV (minimum supported Rust version) of 1.85.

Fix `hr-shape` dependency, so we can publish on crates.io.

## [0.5.1] - 2026-03-04

This release matches HarfBuzz [v13.0.0][harfbuzz-13.0.0], and has an MSRV (minimum supported Rust version) of 1.85.

- New command-line tool `hr-shape` that is a limited counterpart to HarfBuzz `hb-shape`, in its own `hr-shape` crate.
- As a result of the above, source directory turned into a workspace, with new `harfrust` and `hr-shape` directories.
- Fix bug regarding cluster-level=3.
- Various small performance improvements.
- We stand by the people of Iran.

## [0.5.0] - 2026-01-07

This release matches HarfBuzz [v12.3.0][harfbuzz-12.3.0], and has an MSRV (minimum supported Rust version) of 1.85.

- Update to read-fonts 0.37.0 (and bump MSRV to 1.85).
- Various performance improvements.

## [0.4.1] - 2025-12-08

This release matches HarfBuzz [v12.2.0][harfbuzz-12.2.0], and has an MSRV (minimum supported Rust version) of 1.82.

- Make Script::from_iso15924_tag const.
- Avoid panic when saving syllable indices.

## [0.4.0] - 2025-11-10

This release matches HarfBuzz [v12.2.0][harfbuzz-12.2.0], and has an MSRV (minimum supported Rust version) of 1.82.

- Enable more HarfBuzz tests.
- Fix bug from [HarfBust puzzle](https://github.com/harfbuzz/harfbuzz/issues/5535).
- Update to read-fonts 0.36.0.

## [0.3.2] - 2025-10-15

This release matches HarfBuzz [v12.1.0][harfbuzz-12.1.0], and has an MSRV (minimum supported Rust version) of 1.82.

- Fix "would apply" logic for chained sequence context format 3. This bug was preventing accurate classification of
  characters in Indic syllables for some fonts.
- Various optimizations.

## [0.3.1] - 2025-09-12

This release matches HarfBuzz [v11.5.0][harfbuzz-11.5.0], and has an MSRV (minimum supported Rust version) of 1.82.

- Actually bump MSRV from 1.80 to 1.82.

## [0.3.0] - 2025-09-12

This release matches HarfBuzz [v11.5.0][harfbuzz-11.5.0], and has an MSRV (minimum supported Rust version) of 1.82.

- Update to read-fonts 0.35.0.
- Bump MSRV from 1.80 to 1.82.

## [0.2.1] - 2025-09-12

This release matches HarfBuzz [v11.5.0][harfbuzz-11.5.0], and has an MSRV (minimum supported Rust version) of 1.80.

- Update to Unicode 17.0.
- Fix panic when processing chained sequence context format 3.
- Add accessors for script, language and direction to `ShapePlan`.
- Various optimizations.

## [0.2.0] - 2025-08-29

This release matches HarfBuzz [v11.4.4][harfbuzz-11.4.4], and has an MSRV (minimum supported Rust version) of 1.80.

- Major optimizations to speed up AAT shaping.

## [0.1.2] - 2025-08-20

This release matches HarfBuzz [v11.3.3][harfbuzz-11.3.3], and has an MSRV (minimum supported Rust version) of 1.80.

- Major optimizations to speed up shaping.
- Initial support for shape plan caching in the form of `ShapePlanKey`.

## [0.1.1] - 2025-08-11

This release matches HarfBuzz [v11.3.3][harfbuzz-11.3.3], and has an MSRV (minimum supported Rust version) of 1.75.

- Major optimizations to speed up shaping.

## [0.1.0] - 2025-06-10

This release matches HarfBuzz [v11.2.1][harfbuzz-11.2.1], and has an MSRV (minimum supported Rust version) of 1.75.

- Initial Release of HarfRuzz.

HarfRust is a fork of RustyBuzz.
See [their changelog](https://github.com/harfbuzz/rustybuzz/blob/main/CHANGELOG.md) for details of prior releases.

[Unreleased]: https://github.com/harfbuzz/harfrust/compare/0.13.3...HEAD
[0.13.3]: https://github.com/harfbuzz/harfrust/compare/0.13.2...0.13.3
[0.13.2]: https://github.com/harfbuzz/harfrust/compare/0.13.1...0.13.2
[0.13.1]: https://github.com/harfbuzz/harfrust/compare/0.13.0...0.13.1
[0.13.0]: https://github.com/harfbuzz/harfrust/compare/0.12.0...0.13.0
[0.12.0]: https://github.com/harfbuzz/harfrust/compare/0.11.0...0.12.0
[0.11.0]: https://github.com/harfbuzz/harfrust/compare/0.10.0...0.11.0
[0.10.0]: https://github.com/harfbuzz/harfrust/compare/0.9.0...0.10.0
[0.9.0]: https://github.com/harfbuzz/harfrust/compare/0.8.4...0.9.0
[0.8.4]: https://github.com/harfbuzz/harfrust/compare/0.8.3...0.8.4
[0.8.3]: https://github.com/harfbuzz/harfrust/compare/0.8.2...0.8.3
[0.8.2]: https://github.com/harfbuzz/harfrust/compare/0.8.1...0.8.2
[0.8.1]: https://github.com/harfbuzz/harfrust/compare/0.8.0...0.8.1
[0.8.0]: https://github.com/harfbuzz/harfrust/compare/0.7.1...0.8.0
[0.7.1]: https://github.com/harfbuzz/harfrust/compare/0.7.0...0.7.1
[0.7.0]: https://github.com/harfbuzz/harfrust/compare/0.6.2...0.7.0
[0.6.2]: https://github.com/harfbuzz/harfrust/compare/0.6.1...0.6.2
[0.6.1]: https://github.com/harfbuzz/harfrust/compare/0.6.0...0.6.1
[0.6.0]: https://github.com/harfbuzz/harfrust/compare/0.5.2...0.6.0
[0.5.2]: https://github.com/harfbuzz/harfrust/compare/0.5.1...0.5.2
[0.5.1]: https://github.com/harfbuzz/harfrust/compare/0.5.0...0.5.1
[0.5.0]: https://github.com/harfbuzz/harfrust/compare/0.4.1...0.5.0
[0.4.1]: https://github.com/harfbuzz/harfrust/compare/0.4.0...0.4.1
[0.4.0]: https://github.com/harfbuzz/harfrust/compare/0.3.2...0.4.0
[0.3.2]: https://github.com/harfbuzz/harfrust/compare/0.3.1...0.3.2
[0.3.1]: https://github.com/harfbuzz/harfrust/compare/0.3.0...0.3.1
[0.3.0]: https://github.com/harfbuzz/harfrust/compare/0.2.1...0.3.0
[0.2.1]: https://github.com/harfbuzz/harfrust/compare/0.2.0...0.2.1
[0.2.0]: https://github.com/harfbuzz/harfrust/compare/0.1.2...0.2.0
[0.1.2]: https://github.com/harfbuzz/harfrust/compare/0.1.1...0.1.2
[0.1.1]: https://github.com/harfbuzz/harfrust/compare/0.1.0...0.1.1
<!-- The last release of RustyBuzz before 0.1.0. -->
[0.1.0]: https://github.com/harfbuzz/harfrust/compare/8c52723ff75e91a33ae36e527baed871097e64bf...0.1.0

[harfbuzz-11.2.1]: https://github.com/harfbuzz/harfbuzz/releases/tag/11.2.1
[harfbuzz-11.3.3]: https://github.com/harfbuzz/harfbuzz/releases/tag/11.3.3
[harfbuzz-11.4.4]: https://github.com/harfbuzz/harfbuzz/releases/tag/11.4.4
[harfbuzz-11.5.0]: https://github.com/harfbuzz/harfbuzz/releases/tag/11.5.0
[harfbuzz-12.1.0]: https://github.com/harfbuzz/harfbuzz/releases/tag/12.1.0
[harfbuzz-12.2.0]: https://github.com/harfbuzz/harfbuzz/releases/tag/12.2.0
[harfbuzz-12.3.0]: https://github.com/harfbuzz/harfbuzz/releases/tag/12.3.0
[harfbuzz-13.0.0]: https://github.com/harfbuzz/harfbuzz/releases/tag/13.0.0
[harfbuzz-14.1.0]: https://github.com/harfbuzz/harfbuzz/releases/tag/14.1.0

[@khaledhosny]: https://github.com/khaledhosny

[#65]: https://github.com/harfbuzz/harfrust/pull/65
