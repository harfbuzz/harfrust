## Generators

`harfrust/scripts` now only contains generators that are maintained in this
repo. The Unicode/OT table generators shared with HarfBuzz live in `~/harfbuzz/src`
and should be run from there.

Local generators in this repo:

```sh
bash bump-version.sh 0.6.0

python3 ./gen-vowel-constraints.py > ../harfrust/src/hb/ot_shaper_vowel_constraints.rs
rustfmt ../harfrust/src/hb/ot_shaper_vowel_constraints.rs

python3 ./gen-tag-table.py > ../harfrust/src/hb/tag_table.rs
rustfmt ../harfrust/src/hb/tag_table.rs
```

HarfBuzz-owned generators used by `harfrust`:

```sh
PYTHONPATH=/Users/behdad/packtab \
python3 ~/harfbuzz/src/gen-ucd-table.py --rust \
  ~/harfbuzz/src/ucd.nounihan.grouped.xml \
  ~/harfbuzz/src/hb-script-list.h \
  > ../harfrust/src/hb/ucd_table.rs

PYTHONPATH=/Users/behdad/packtab \
python3 ~/harfbuzz/src/gen-use-table.py --rust \
  ~/harfbuzz/src/IndicSyllabicCategory.txt \
  ~/harfbuzz/src/IndicPositionalCategory.txt \
  ~/harfbuzz/src/ArabicShaping.txt \
  ~/harfbuzz/src/DerivedCoreProperties.txt \
  ~/harfbuzz/src/UnicodeData.txt \
  ~/harfbuzz/src/Blocks.txt \
  ~/harfbuzz/src/Scripts.txt \
  ./ms-use/IndicSyllabicCategory-Additional.txt \
  ./ms-use/IndicPositionalCategory-Additional.txt \
  > ../harfrust/src/hb/ot_shaper_use_table.rs

PYTHONPATH=/Users/behdad/packtab \
python3 ~/harfbuzz/src/gen-arabic-table.py --rust \
  ~/harfbuzz/src/ArabicShaping.txt \
  ~/harfbuzz/src/UnicodeData.txt \
  ~/harfbuzz/src/Blocks.txt \
  > ../harfrust/src/hb/ot_shaper_arabic_table.rs

PYTHONPATH=/Users/behdad/packtab \
python3 ~/harfbuzz/src/gen-indic-table.py --rust \
  ~/harfbuzz/src/IndicSyllabicCategory.txt \
  ~/harfbuzz/src/IndicPositionalCategory.txt \
  ~/harfbuzz/src/Blocks.txt \
  > ../harfrust/src/hb/ot_shaper_indic_table.rs

PYTHONPATH=/Users/behdad/packtab \
python3 ~/harfbuzz/src/gen-emoji-table.py --rust \
  ~/harfbuzz/src/emoji-data.txt \
  ~/harfbuzz/src/emoji-test.txt \
  > ../harfrust/src/hb/unicode_emoji_table.rs
```
