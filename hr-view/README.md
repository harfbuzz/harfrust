# hr-view

Command-line text rendering utility for the HarfRust library, equivalent to HarfBuzz's `hb-view`.

Use `hr-view` to render text with a font to a PNG image. It shapes with
HarfRust and draws the resulting glyph outlines, so it shows what the shaping
engine produced, including the output of a font's own WebAssembly shaper.

## Installation

```bash
cargo install hr-view
```

To render fonts whose shaping comes from an embedded `Wasm` table, build with
the `wasm` feature:

```bash
cargo install hr-view --features wasm
```

## Usage

```bash
hr-view [OPTIONS] <font-file> <text>
```

The image is written to stdout (or displayed in a supported terminal) by default. Use `-o` to write to a file, or `-o -` to explicitly write to stdout.

```bash
hr-view font.ttf "Hello, world!" -o hello.png --font-size=128
hr-view font.ttf "abc" --background=1e1e2e --foreground=cdd6f4 -o dark.png
hr-view font.ttf $'first line\nsecond line' -o lines.png
hr-view wasm-font.ttf "text" --shapers=wasm -o wasm.png
hr-view font.ttf "text" > preview.png
```

Text is taken from the second argument, `--text`, `--text-file` (`-` for
stdin), or `--unicodes` (`U+0056,U+0057`), in that order. Options for the
direction, script, language, features, variations and point size follow
`hr-shape`:

```text
--font-file, --face-index, --font-size, --font-ptem, --variations,
--named-instance, --direction, --language, --script, --features, --shapers
```

`--shapers` takes a comma-separated list of shapers to try, in order, and
defaults to `ot`. `--list-shapers` prints the shapers this build supports.
`--font-size` is in pixels and defaults to 256; give separate x,y values for
different horizontal and vertical scales.

Rendering options:

```text
-o, --output-file <FILE>     Output file (use "-" for stdout) [default: stdout/terminal]
    --margin <PIXELS>        Border around the text [default: 16]
    --background <COLOR>     rrggbb or rrggbbaa, or "none" [default: ffffff]
    --foreground <COLOR>     rrggbb or rrggbbaa [default: 000000]
    --line-space <PIXELS>    Extra space between lines [default: 0]
```

The image is sized to the text: the ink extents of the drawn glyphs together
with the font's line box, plus the margin.

**Note:** `hr-view` renders only glyph outlines. Glyphs with color fonts (COLR,
CBDT) or other bitmap formats are not rendered and produce a warning.

For more information, see the main [HarfRust repository](https://github.com/harfbuzz/harfrust).
