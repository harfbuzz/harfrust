"""Build an original variable GPOS fixture for OpenType rounding tests.

Run with fontTools installed. The generated font uses the repository's MIT
license. At wght=0.25, every positioning delta for the A B pair is -2.5.
"""

from pathlib import Path

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables import otTables
from fontTools.varLib.builder import (
    buildVarData, buildVarDevTable, buildVarRegionList, buildVarStore,
)


builder = FontBuilder(1000, isTTF=True)
names = [".notdef", "A", "B"]
builder.setupGlyphOrder(names)
builder.setupCharacterMap({ord("A"): "A", ord("B"): "B"})
builder.setupGlyf({name: TTGlyphPen(None).glyph() for name in names})
builder.setupHorizontalMetrics({name: (500, 0) for name in names})
builder.setupHorizontalHeader(ascent=800, descent=-200)
builder.setupVerticalMetrics({name: (1000, 0) for name in names})
builder.setupVerticalHeader(ascent=500, descent=-500)
builder.setupNameTable({"familyName": "Rounding Test", "styleName": "Regular"})
builder.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
builder.setupPost()
builder.setupFvar([("wght", 0, 0, 1, "Weight")], [])
font = builder.font
addOpenTypeFeaturesFromString(font, """
    languagesystem DFLT dflt;
    lookup Pair {
        pos A B <10 10 10 10>;
    } Pair;
    feature kern { lookup Pair; } kern;
    feature vkrn { lookup Pair; } vkrn;
""")
value = font["GPOS"].table.LookupList.Lookup[0].SubTable[0].PairSet[0].PairValueRecord[0].Value1
for field in ["XPlaDevice", "YPlaDevice", "XAdvDevice", "YAdvDevice"]:
    setattr(value, field, buildVarDevTable(0))
font["GPOS"].table.LookupList.Lookup[0].SubTable[0].ValueFormat1 = value.getFormat()
gdef = newTable("GDEF")
gdef.table = otTables.GDEF()
gdef.table.Version = 0x00010003
gdef.table.VarStore = buildVarStore(
    buildVarRegionList([{"wght": (0, 1, 1)}], ["wght"]),
    [buildVarData([0], [[-10]])],
)
font["GDEF"] = gdef
font["head"].created = font["head"].modified = 3400000000
font.recalcTimestamp = False
font.save(Path(__file__).with_name("rounding.ttf"))
