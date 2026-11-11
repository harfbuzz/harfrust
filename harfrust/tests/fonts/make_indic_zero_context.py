"""Build an original fixture for Indic zero-context would_substitute matching.

Run with fontTools installed. The generated font uses the repository's MIT
license. Each script's 'blwf' lookup forms a below-base KA only after a KA,
so it would substitute only if backtrack context is allowed: for 'beng'
(old spec) and 'mlm2' (Malayalam), but not for 'ory2' (new spec).
"""

from pathlib import Path

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen


scripts = [("ory2", "ori", 0x0B15), ("beng", "bng", 0x0995), ("mlm2", "mlm", 0x0D15)]
names = [".notdef"]
cmap = {}
for _, prefix, ka in scripts:
    names += [f"ka.{prefix}", f"virama.{prefix}", f"ka.below.{prefix}"]
    cmap[ka] = f"ka.{prefix}"
    cmap[ka + 0x38] = f"virama.{prefix}"

builder = FontBuilder(1000, isTTF=True)
builder.setupGlyphOrder(names)
builder.setupCharacterMap(cmap)
builder.setupGlyf({name: TTGlyphPen(None).glyph() for name in names})
builder.setupHorizontalMetrics(
    {name: (0 if "virama" in name or "below" in name else 500, 0) for name in names}
)
builder.setupHorizontalHeader(ascent=800, descent=-200)
builder.setupNameTable({"familyName": "Indic Zero Context Test", "styleName": "Regular"})
builder.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
builder.setupPost()
font = builder.font
fea = ["languagesystem DFLT dflt;"]
fea += [f"languagesystem {tag} dflt;" for tag, _, _ in scripts]
for tag, p, _ in scripts:
    fea.append(f"""
lookup BelowKa_{p} {{
    sub virama.{p} ka.{p} by ka.below.{p};
}} BelowKa_{p};
lookup Blwf_{p} {{
    sub ka.{p} virama.{p}' lookup BelowKa_{p} ka.{p}';
}} Blwf_{p};
feature blwf {{
    script {tag};
    lookup Blwf_{p};
}} blwf;""")
addOpenTypeFeaturesFromString(font, "\n".join(fea))
font["head"].created = font["head"].modified = 3400000000
font.recalcTimestamp = False
font.save(Path(__file__).with_name("indic-zero-context.ttf"))
