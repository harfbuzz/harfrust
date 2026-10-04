`BaselineDevice.otf` is derived from
`harfrust/tests/fonts/rb_custom/NotoSansCJK.subset1.otf` under the accompanying
`NotoSans-OFL.txt` license. Its default-script `icfb` BASE coordinate was changed
to format 3 with a Device table covering ppem 12 through 17 and deltas 1
through 6. It tests that the C font's ppem settings affect baseline queries.

`MathQuery.ttf` is derived from
`harfrust/tests/fonts/in-house/8d9c4b193808b8bde94389ba7831c1fc6f9e794e.ttf`
by `make_math_query.py`. It adds one covered glyph for each MATH query and
Device adjustments to AxisHeight and the kern height and value.
