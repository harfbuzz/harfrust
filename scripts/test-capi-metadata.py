#!/usr/bin/env python3
"""Compare Fontations-backed C metadata queries with HarfBuzz (14.5.1+).

Build harfrust_capi, then supply shared libraries and a font directory:
  python3 scripts/test-capi-metadata.py --library /path/to/libharfrust_c.so \
      --harfbuzz /path/to/libharfbuzz.so --fonts harfrust/tests/fonts
"""
import argparse
import ctypes as C
from pathlib import Path
import struct

U = C.c_uint
P = C.c_void_p
UP = C.POINTER(U)


def tag(value):
    return int.from_bytes(value.encode("ascii"), "big")


def sfnt(table, data):
    return struct.pack(">IHHHH4sIII", 0x10000, 1, 16, 0, 0, table.encode("ascii"),
                       0, 28, len(data)) + data


class Api:
    def __init__(self, library, prefix):
        self.library = C.CDLL(library)
        self.prefix = prefix
        self.blob_create = self.bind("blob_create", P, [P, U, C.c_int, P, P])
        self.blob_destroy = self.bind("blob_destroy", None, [P])
        self.face_create = self.bind("face_create", P, [P, U])
        self.face_destroy = self.bind("face_destroy", None, [P])
        self.face_count = self.bind("face_count", U, [P])
        self.palettes = self.bind("ot_color_has_palettes", C.c_int, [P])
        self.palette_count = self.bind("ot_color_palette_get_count", U, [P])
        self.palette_flags = self.bind("ot_color_palette_get_flags", U, [P, U])
        self.palette_colors = self.bind("ot_color_palette_get_colors", U, [P, U, U, UP, UP])

    def bind(self, name, result, arguments):
        function = getattr(self.library, self.prefix + name)
        function.restype = result
        function.argtypes = arguments
        return function

    def face(self, data, index):
        blob = self.blob_create(data, len(data), 0, None, None)
        face = self.face_create(blob, index)
        self.blob_destroy(blob)
        return face


def colors(api, face, palette, start, capacity, array=True, count=True):
    size = U(capacity)
    output = (U * (capacity + 1))(*([0xDEADBEEF] * (capacity + 1)))
    total = api.palette_colors(face, palette, start, C.byref(size) if count else None,
                               output if array else None)
    return total, size.value, list(output)


def synthetic_fonts():
    for version in [0, 1]:
        color_offset = 16 if version == 0 else 28
        data = struct.pack(">HHHHIHH", version, 3, 2, 5, color_offset, 0, 2)
        if version:
            data += struct.pack(">III", 48, 0, 0)
        data += struct.pack(">IIIII", 0x12345601, 0xABCDEF00, 0x000000FF, 0x10203040, 0xF0E0D0C0)
        if version:
            data += struct.pack(">II", 1, 2)
        yield f"synthetic CPAL v{version}", sfnt("CPAL", data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True)
    parser.add_argument("--harfbuzz", required=True)
    parser.add_argument("--fonts", type=Path, required=True)
    args = parser.parse_args()
    hr, hb = Api(args.library, "hr_"), Api(args.harfbuzz, "hb_")
    version = hb.bind("version_atleast", C.c_int, [U, U, U])
    if not version(14, 5, 1):
        parser.error("use HarfBuzz >= 14.5.1 for nullable pagination tests")
    checks = faces = 0

    def equal(label, actual, expected):
        nonlocal checks
        checks += 1
        assert actual == expected, (label, actual, expected)

    fonts = list(synthetic_fonts())
    fonts.extend((str(p), p.read_bytes()) for p in sorted(args.fonts.rglob("*"))
                 if p.suffix.lower() in {".ttf", ".otf", ".ttc"})
    for name, data in fonts:
        blob = hb.blob_create(data, len(data), 0, None, None)
        count = hb.face_count(blob)
        hb.blob_destroy(blob)
        for index in range(count):
            hrf, hbf = hr.face(data, index), hb.face(data, index)
            faces += 1
            try:
                equal((name, "has_palettes"), hr.palettes(hrf), hb.palettes(hbf))
                total = hb.palette_count(hbf)
                equal((name, "palette_count"), hr.palette_count(hrf), total)
                for palette in [*range(total), total, 0xFFFFFFFF]:
                    equal((name, "flags", palette), hr.palette_flags(hrf, palette),
                          hb.palette_flags(hbf, palette))
                    entries = hb.palette_colors(hbf, palette, 0, None, None)
                    for start, capacity, array, count in [
                        (0, entries, True, True), (0, 1, True, True),
                        (1, 3, True, True), (entries, 2, True, True),
                        (0xFFFFFFFF, 2, True, True), (0, 99, False, True),
                        (0, 99, True, False), (0, 0, True, True),
                    ]:
                        equal((name, "colors", palette, start, capacity, array, count),
                              colors(hr, hrf, palette, start, capacity, array, count),
                              colors(hb, hbf, palette, start, capacity, array, count))
            finally:
                hr.face_destroy(hrf)
                hb.face_destroy(hbf)
    print(f"Matched HarfBuzz in {checks} metadata comparisons across {faces} font faces.")


if __name__ == "__main__":
    main()
