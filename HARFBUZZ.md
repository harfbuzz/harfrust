# Comparing to HarfBuzz for correctness & performance

We aspire to match HarfBuzz, in correctness and performance.

We have our own subset of the HarfBuzz shaping tests in `tests`,
which all pass (`cargo test`).

For performance comparisons through the C APIs, use
[benchmark-harf](https://github.com/harfbuzz/benchmark-harf), described below.
We also have a Rust benchmark suite with `cargo bench` that compares against the
[`harfbuzz_rs`](https://github.com/harfbuzz/harfbuzz_rs) bindings. These measure
different API paths and should be reported separately.

This document covers direct C API comparisons and HarfBuzz's own shaping tests
and benchmarks using its HarfRust integration.

## Building HarfBuzz with HarfRust support

First, HarfBuzz requires the Rust nightly toolchain to build
with HarfRust support. It also requires bindgen. You can prepare
those all using:

```sh
$ rustup default nightly
$ rustup component add rust-src
$ cargo install bindgen-cli
```

Next, we need to build HarfBuzz with HarfRust shaper support
and the benchmark suite enabled. We also build in `release` mode.
By default HarfBuzz builds with a `debugoptimized` mode, that
is slightly slower, but better for development.

```sh
$ meson build -Dbenchmark=enabled -Dharfrust=enabled -Dbuildtype=release
$ ninja -C build
```


## Running HarfBuzz's Shaping Tests

This one is easy: we override the default shaper via an environment
variable, and then run the tests.  Note that you have to use `meson`
to run the tests; it won't work with `ninja` directly, since `ninja`
locks the environment variables.

```sh
$ HB_SHAPER_LIST=harfrust meson test -C build
```

The exact totals change as HarfBuzz adds tests. As of August 24, 2026,
the relevant shaping suites report:

```
shape+aots:                 800/800 passed
shape+text-rendering-tests: 437/437 passed
shape+in-house:             5012/5021 passed
```

To see specific failures, you can run inspect the test log:
```sh
$ less build/meson-logs/testlog.txt
```

The nine remaining shaping failures are all in
`arabic-fallback-shaping.tests`; HarfRust does not yet synthesize Arabic
fallback lookups for fonts without the required OpenType substitutions.


## Comparing the C APIs with benchmark-harf

[benchmark-harf](https://github.com/harfbuzz/benchmark-harf) builds one C++
Google Benchmark executable with the same shaping loop compiled in separate
translation units against HarfBuzz's `<hb.h>` and HarfRust's `<hr-hb.h>`
compatibility header. It links both libraries into one process. This keeps the
calling code and C API usage consistent, avoiding differences between a Rust
benchmark's native Rust path and its HarfBuzz bindings.

The result suffixes identify three distinct paths:

- `/hb`: HarfBuzz's native OpenType/AAT shaper (named `ot` internally).
- `/hr`: HarfRust through its C API.
- `/hb-hr`: HarfRust through HarfBuzz's integration, registered only when that
  HarfBuzz build supports it.

Use `/hb` and `/hr` for a direct C API comparison. The `/hb-hr` path measures
HarfBuzz's integration overhead as well as HarfRust shaping; it may also use a
different HarfRust revision or dependency build from the standalone C library.
Both engines support OpenType and AAT shaping.

### Building and running

Install a C++ compiler, Rust, Meson, Ninja, GLib development files, and Google
Benchmark (or allow Meson to fetch its fallback). The following example uses
sibling source directories `~/hb`, `~/hr`, and `~/benchmark-harf`. It creates
separate release build directories; substitute your source paths as needed.

```sh
# Build HarfBuzz's native library; its HarfRust integration is optional.
CXX=clang++ CC=clang meson setup "$HOME/hb/build-capi-bench" "$HOME/hb" \
  --buildtype=release -Db_lto=true -Dharfrust=disabled
meson compile -C "$HOME/hb/build-capi-bench"

# Run from the source tree so Cargo also reads its .cargo/config.toml.
# HarfRust's workspace release profile enables fat LTO and one codegen unit.
cd "$HOME/hr"
cargo build --target-dir "$HOME/hr/target-capi-bench" \
  -p harfrust_capi --release

git clone https://github.com/harfbuzz/benchmark-harf.git "$HOME/benchmark-harf"
CXX=clang++ CC=clang meson setup "$HOME/benchmark-harf/build" \
  "$HOME/benchmark-harf" --buildtype=release -Db_lto=true \
  -Dhb_root="$HOME/hb" -Dhb_build="$HOME/hb/build-capi-bench" \
  -Dhr_root="$HOME/hr" -Dhr_build="$HOME/hr/target-capi-bench/release"
meson compile -C "$HOME/benchmark-harf/build"

# Compare both native C API paths on an identical font/text pair.
"$HOME/benchmark-harf/build/benchmark-harf" \
  "$HOME/hb/perf/fonts/Roboto-Regular.ttf" \
  "$HOME/hb/perf/texts/en-thelittleprince.txt" \
  --benchmark_filter='/(hb|hr)$' \
  --benchmark_min_time=0.5s --benchmark_repetitions=9 \
  --benchmark_enable_random_interleaving=true \
  --benchmark_out=results.json --benchmark_out_format=json
```

The harness uses fonts and corpora from the configured HarfBuzz source tree
when run without explicit font/text pairs. You can pass multiple pairs, or
select benchmarks with `--benchmark_filter`. See its README for the current
workload list and optional local AAT font inputs. By default, available AAT
collections are read from the configured HarfBuzz source root; use
`--aat-font-dir=/path/to/local/fonts` to select another directory. Missing
optional fonts are skipped, and explicit font/text pairs replace all defaults.
Proprietary fonts must be supplied locally and are not distributed with the
harness. For variable fonts,
use `--variations='wght=700'`; use `--direction=rtl` (or another direction) to
override the direction guessed from the text.

### Making a fair comparison

Each timed iteration shapes the entire corpus one newline-delimited line at a
time. The final line must end with a newline to be included. The harness loads
the font and text and performs one warm-up pass before timing; it reuses the
font and buffer across lines and iterations. Results are milliseconds per
complete corpus, not per line or per glyph, and describe warm shaping rather
than font loading or cold startup.

Use identical font bytes, collection face, text bytes, line boundaries,
direction, script, language, features, variations, font scale, and buffer
settings. The current harness uses face index zero, default features, and
guessed segment properties unless direction is overridden. Extend both
compiled paths together if you need other settings. Check output parity
(glyph IDs, clusters, advances, and offsets) separately before interpreting
timings: the benchmark does not compare glyph output. Exclude and report
inputs for which either engine fails, reaches a shaping budget, or returns
incomplete output; a quick failed or truncated shape is not a performance win.

Build both libraries in release mode with comparable optimization and LTO
settings. Record the C++ and Rust compiler versions, LLVM versions, target CPU
flags, Cargo features, exact HarfBuzz/HarfRust/Fontations revisions, local
patches, and benchmark-harf revision. Compiler choice and target tuning can
change the ranking; linking into the same process does not remove those
compiler differences. Check which libraries the executable actually loads,
particularly after changing build paths or environment variables.

Keep the machine otherwise idle, use a stable CPU frequency policy, and pin
the process to one CPU where practical (for example, `taskset -c 2` on Linux).
Use repeated, interleaved runs and compare the median CPU times in the JSON
output, retaining the individual repetitions to assess variation. Report
workloads separately rather than treating one corpus as representative of all
shaping. Current results are workload dependent: common Latin cases can be
close, while either engine can lead on other fonts and scripts. The
[historical spreadsheet](https://docs.google.com/spreadsheets/d/1lyPPZHXIF8gE0Tpx7_IscwhwaZa4KOpdt7vnV0jQT9o/preview)
contains earlier comparisons; its timings are not a substitute for measuring
the revisions and configuration you plan to use.

## Running HarfBuzz's Benchmark Tests

HarfBuzz's `build/perf/benchmark-shape` remains useful for measuring its shaper
integration. Build with `-Dbenchmark=enabled -Dharfrust=enabled` as above, then
compare its native `ot` path with the `harfrust` path:

```sh
build/perf/benchmark-shape --benchmark_filter='/(ot|harfrust)$' \
  --benchmark_repetitions=9 --benchmark_out=integration-results.json \
  --benchmark_out_format=json
```

It also accepts custom font/text pairs, for example:

```sh
build/perf/benchmark-shape Gulzar-Regular.ttf perf/texts/fa-words.txt \
  --benchmark_filter='/(ot|harfrust)$'
```

Read the CPU column and apply the same workload, build, output-parity, and
measurement controls described above. Label these results as the HarfBuzz
integration path, rather than direct HarfRust C API timings.


## Running against local HarfRust and Fontations

By default, HarfBuzz will use HarfRust from github `main` branch,
as can bee seen in `src/rust/Cargo.toml` file:
```toml
[dependencies]
skrifa = { version = "0.*", optional = true }
harfrust = { git = "https://github.com/harfbuzz/harfrust", optional = true }
```

To run against a different branch, just add the branch name as an
extra configuration to `src/rust/Cargo.toml`, eg.:
```toml
[dependencies]
skrifa = { version = "0.*", optional = true }
harfrust = { branch = "NAME", git = "https://github.com/harfbuzz/harfrust", optional = true }
```

To run against a local HarfRust, you can use a local path, eg.:
```toml
[dependencies]
skrifa = { version = "0.*", optional = true }
harfrust = { path = "../../../harfrust", optional = true }
```

Note that to run HarfRust against a local Fontations, you need to
also modify the HarfRust `Cargo.toml` file to point to the local
`fontations` path. Look for the `read-fonts` under `[dependencies]` section:
```toml
read-fonts = { path = "../fontations/read-fonts", default-features = false, features = ["libm"] }
```

Finally, HarfBuzz wouldn't see any changes you make to the HarfRust
or Fontations code, so it would not rebuild them. You can force
HarfBuzz to rebuild by, eg., touching the HarfBuzz rust shaper file:
```sh
$ touch src/rust/shaper.rs && ninja -Cbuild
```
or faster, if you just want to run the benchmark:
```sh
$ touch src/rust/shaper.rs && ninja -Cbuild perf/benchmark-shape
```
