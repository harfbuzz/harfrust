# hr-shape

Command-line text shaping utility for the HarfRust library, equivalent to HarfBuzz's `hb-shape`.

Use `hr-shape` to shape text using fonts and inspect the shaping results.

## Installation

```bash
cargo install hr-shape
```

## Usage

```bash
hr-shape [OPTIONS] <font-file> <text>
```

Pass `--trace` to print HarfBuzz shaping messages and intermediate buffer
contents to stderr. Glyph results go to stdout or the requested output file.
The library's `render` API includes traces in its returned string.

## Choosing a shaper

`--shapers` takes a comma-separated list of shapers to try, in order, and
defaults to `ot`. `--list-shapers` prints the shapers this build supports.

```bash
hr-shape --list-shapers                    # ot, wasm
hr-shape font.ttf "text" --shapers=ot      # OpenType (the default)
hr-shape font.ttf "text" --shapers=wasm    # the font's own WebAssembly module
hr-shape font.ttf "text" --shapers=wasm,ot # module if the font has one
```

The WebAssembly shaper needs the `wasm` feature and
a font with a `Wasm` table. A shaper that cannot shape the font is passed over,
so `--shapers=wasm` on a font without a module fails while `--shapers=wasm,ot`
falls back to OpenType. See [docs/wasm-shaper.md](../docs/wasm-shaper.md).

For more information, see the main [HarfRust repository](https://github.com/harfbuzz/harfrust).
