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
contents before the final glyph result.

For more information, see the main [HarfRust repository](https://github.com/harfbuzz/harfrust).
