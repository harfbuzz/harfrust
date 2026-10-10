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

For more information, see the main [HarfRust repository](https://github.com/harfbuzz/harfrust).
