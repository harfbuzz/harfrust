#!/usr/bin/env python3
"""Writes ligature-mark-walk.ttf to the path given as the only argument.

Regression fixture for the ligature base walk in match_input(): a match that
starts at a mark of an earlier ligature walks the output buffer back over
every mark of that ligature. The font makes that walk happen many times at
one glyph, so that it must be charged to the operation budget.

Characters a..g map to glyphs 1..7. GDEF makes c a mark, every other glyph a
base. GSUB feature 'liga' runs lookups 0 and 1:
  0  LigatureSubst, IgnoreMarks  a b -> e   every c skipped between a and b
                                            becomes a mark of ligature e
  1  ChainContext   c, lookahead d          100 records, each applying lookup 2
  2  ChainContext   c                       100 records, each applying lookup 3
  3  LigatureSubst  c d g -> f              tried 100 x 100 times at the last c
                                            of "a c...c b d"; never matches
"""
import struct
import sys

A, B, C, D, E, F, G = range(1, 8)
NUM_GLYPHS = 8
RECORDS_PER_CONTEXT = 100
IGNORE_MARKS = 0x0008
BASE, MARK = 1, 3


def u16(*values):
    return struct.pack(">%dH" % len(values), *values)


def coverage(glyph):
    return u16(1, 1, glyph)


def lookup(lookup_type, flags, subtable):
    return u16(lookup_type, flags, 1, 8) + subtable


def ligature_subst(first, rest, ligature_glyph):
    ligature = u16(ligature_glyph, 1 + len(rest), *rest)
    ligature_set = u16(1, 4) + ligature
    # coverage right after the 8-byte header, the one ligature set after the 6-byte coverage
    return u16(1, 8, 1, 14) + coverage(first) + ligature_set


def chain_context(input_glyph, lookahead_glyphs, nested_lookup):
    records = u16(0, nested_lookup) * RECORDS_PER_CONTEXT
    header_size = 12 + 2 * len(lookahead_glyphs) + len(records)
    coverages = [coverage(g) for g in [input_glyph] + lookahead_glyphs]
    offsets = [header_size + 6 * i for i in range(len(coverages))]
    # format 3: no backtrack, one input glyph, then the lookahead and the lookup records
    header = u16(3, 0, 1, offsets[0], len(lookahead_glyphs), *offsets[1:], RECORDS_PER_CONTEXT) + records
    return header + b"".join(coverages)


lookups = [
    lookup(4, IGNORE_MARKS, ligature_subst(A, [B], E)),
    lookup(6, 0, chain_context(C, [D], 2)),
    lookup(6, 0, chain_context(C, [], 3)),
    lookup(4, 0, ligature_subst(C, [D, G], F)),
]
lookup_offsets, lookup_bodies = [], b""
for table in lookups:
    lookup_offsets.append(2 + 2 * len(lookups) + len(lookup_bodies))
    lookup_bodies += table
lookup_list = u16(len(lookups), *lookup_offsets) + lookup_bodies
script_list = u16(1) + b"DFLT" + u16(8) + u16(4, 0) + u16(0, 0xFFFF, 1, 0)
feature_list = u16(1) + b"liga" + u16(8) + u16(0, 2, 0, 1)
gsub = u16(1, 0, 10, 10 + len(script_list), 10 + len(script_list) + len(feature_list))
gsub += script_list + feature_list + lookup_list

glyph_classes = u16(1, A, G - A + 1, *[MARK if glyph == C else BASE for glyph in range(A, G + 1)])
gdef = u16(1, 0, 12, 0, 0, 0) + glyph_classes

cmap_format4 = u16(4, 32, 0, 4, 4, 1, 0)
cmap_format4 += u16(ord("g"), 0xFFFF, 0, ord("a"), 0xFFFF)
cmap_format4 += u16((A - ord("a")) & 0xFFFF, 1, 0, 0)
cmap = u16(0, 1, 3, 1, 0, 12) + cmap_format4

tables = {
    b"GDEF": gdef,
    b"GSUB": gsub,
    b"cmap": cmap,
    b"head": struct.pack(">HHIIIHHQQhhhhHHhhh", 1, 0, 0x10000, 0, 0x5F0F3CF5, 0, 1000, 0, 0, 0, 0, 0, 0, 0, 8, 2, 0, 0),
    b"hhea": struct.pack(">HHhhhHhhhhhhhhhhhH", 1, 0, 800, -200, 0, 500, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1),
    b"hmtx": struct.pack(">Hh", 500, 0) + b"\0\0" * (NUM_GLYPHS - 1),
    b"maxp": struct.pack(">IH", 0x00005000, NUM_GLYPHS),
}
directory = struct.pack(">IHHHH", 0x00010000, len(tables), 0, 0, 0)
table_data = b""
for tag in sorted(tables):
    directory += struct.pack(">4sIII", tag, 0, 12 + 16 * len(tables) + len(table_data), len(tables[tag]))
    table_data += tables[tag] + b"\0" * (-len(tables[tag]) % 4)

with open(sys.argv[1], "wb") as font_file:
    font_file.write(directory + table_data)
