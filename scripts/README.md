## Generators

`harfrust/scripts` now only contains generators that are maintained in this
repo. The Unicode/OT table generators shared with HarfBuzz live in `~/harfbuzz/src`
and should be run from there.

For regenerating the Rust-side shared Unicode/OT tables, use the wrapper
makefile in this directory. It assumes HarfBuzz lives in `~/harfbuzz` by
default and can be overridden with `HARFBUZZ_DIR=...`.

Local generators in this repo:

```sh
bash bump-version.sh 0.6.0

python3 ./gen-shaping-tests.py

python3 ./gen-shaping-tests.py /path/to/harfbuzz
```

`gen-shaping-tests.py` assumes HarfBuzz is in `~/harfbuzz` by default. If
`~/harfbuzz/builddir/util/hb-shape` is missing but `~/harfbuzz/builddir`
exists, it will run `ninja -C ~/harfbuzz/builddir` first.

HarfBuzz-owned generators used by `harfrust`:

```sh
make -f update-unicode-tables.mk

make -f update-unicode-tables.mk hb-refresh

make -f update-unicode-tables.mk HARFBUZZ_DIR=/path/to/harfbuzz
```
