# State machines generation using ragel

HarfBuzz uses [ragel](https://github.com/adrian-thurston/colm-suite) to generate
state machines for some shapers.
The produced C code is rather low-level and relies on `goto` a lot,
therefore converting it to Rust by hand is rather problematic.
And this is the code that updates relatively often.

Ragel's `main` branch has native Rust support. Use the latest development version
from the unified `colm-suite` repository to regenerate the Rust state machines.

## Build ragel

`ragel` is a C++ project that uses `autotools`. The suite builds its Colm
dependency first, so a separate Colm checkout and installation are unnecessary.

In case of macOS, we would need:

```sh
brew install automake autoconf libtool
```

And now we can build `ragel`:

```sh
git clone --branch main https://github.com/adrian-thurston/colm-suite
cd colm-suite
./autogen.sh
./configure --prefix=/path/to/ragel/install --disable-manual
make
make install
```

## Running ragel

From the HarfRust repository root, regenerate all four machines using:

```sh
RAGEL_RUST=/path/to/ragel/install/bin/ragel-rust
for machine in indic myanmar use; do
    "$RAGEL_RUST" -e -F1 "harfrust/src/ot/shaper/${machine}_machine.rl"
done
"$RAGEL_RUST" -e -T0 harfrust/src/ot/shaper/khmer_machine.rl
```

Khmer uses the table-driven `-T0` backend; the other machines use the flat
table-driven `-F1` backend.

The machines were last regenerated with Ragel 7.1.0 from
[`colm-suite` commit `42bc3a875564`](https://github.com/adrian-thurston/colm-suite/commit/42bc3a87556413cafd60cb6bc5e9fa034d2a364a).

Ragel removes temporary `*.ri` files after successful generation.

## Code format

`ragel` uses strange code formatting for the Rust output, therefore running `cargo fmt`
is required.

## The Universal state machine

The `use_machine.rl` is special since we have to modify its output manually
because we cannot express our needs via `ragel` directly.

After `use_machine.rs` is generated, replace assignments of `0` to `p`, `ts`,
or `te` with `p0`, as indicated by the comment in the source. These variables
are `MachineCursor`s rather than integer offsets. Then run `cargo fmt --all`
and `cargo test -p harfrust`.
