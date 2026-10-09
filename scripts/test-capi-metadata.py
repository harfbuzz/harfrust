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


class AxisInfo(C.Structure):
    _fields_ = [("axis_index", U), ("tag", U), ("name_id", U), ("flags", U),
                ("min_value", C.c_float), ("default_value", C.c_float),
                ("max_value", C.c_float), ("reserved", U)]


class SelectorInfo(C.Structure):
    _fields_ = [("name_id", U), ("enable", U), ("disable", U), ("reserved", U)]


class NameEntry(C.Structure):
    _fields_ = [("name_id", U), ("var", U), ("language", P)]


def tag(value):
    return int.from_bytes(value.encode("ascii"), "big")


def sfnt(table, data):
    return sfnt_tables({table: data})


def sfnt_tables(tables):
    count = len(tables)
    power = count.bit_length() - 1
    result = struct.pack(">IHHHH", 0x10000, count, 16 << power, power, count * 16 - (16 << power))
    offset = 12 + 16 * count
    records, storage = b"", b""
    for table, data in sorted(tables.items()):
        records += struct.pack(">4sIII", table.encode("ascii"), 0, offset, len(data))
        padded = data + bytes((-len(data)) % 4)
        storage += padded
        offset += len(padded)
    return result + records + storage


class Api:
    def __init__(self, library, prefix):
        self.library = C.CDLL(library)
        self.prefix = prefix
        self.blob_create = self.bind("blob_create", P, [P, U, C.c_int, P, P])
        self.blob_destroy = self.bind("blob_destroy", None, [P])
        self.face_create = self.bind("face_create", P, [P, U])
        self.face_destroy = self.bind("face_destroy", None, [P])
        self.face_count = self.bind("face_count", U, [P])
        self.feature_types = self.bind("aat_layout_get_feature_types", U, [P, U, UP, UP])
        self.selectors = self.bind("aat_layout_feature_type_get_selector_infos", U,
                                   [P, U, U, UP, C.POINTER(SelectorInfo), UP])
        self.axis_count = self.bind("ot_var_get_axis_count", U, [P])
        self.axes = self.bind("ot_var_get_axis_infos", U, [P, U, UP, C.POINTER(AxisInfo)])
        self.language = self.bind("language_from_string", P, [C.c_char_p, C.c_int])
        self.name = self.bind("ot_name_get_utf16", U, [P, U, P, UP, C.POINTER(C.c_uint16)])
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


def axes(api, face, start, capacity, array=True, count=True):
    size = U(capacity)
    output = (AxisInfo * (capacity + 1))()
    C.memset(output, 0xCD, C.sizeof(output))
    total = api.axes(face, start, C.byref(size) if count else None, output if array else None)
    return total, size.value, bytes(output)


def name_text(api, face, name_id, language, capacity):
    size = U(capacity)
    output = (C.c_uint16 * (capacity + 1))(*([0xDEAD] * (capacity + 1)))
    language = api.language(language, -1) if language else None
    total = api.name(face, name_id, language, C.byref(size), output)
    return total, size.value, list(output)


def feature_types(api, face, start, capacity, array=True, count=True):
    size = U(capacity)
    output = (U * (capacity + 1))(*([0xDEADBEEF] * (capacity + 1)))
    total = api.feature_types(face, start, C.byref(size) if count else None,
                              output if array else None)
    return total, size.value, list(output)


def selectors(api, face, feature, start, capacity, array=True, count=True):
    size, default = U(capacity), U(123)
    output = (SelectorInfo * (capacity + 1))()
    C.memset(output, 0xCD, C.sizeof(output))
    total = api.selectors(face, feature, start, C.byref(size) if count else None,
                          output if array else None, C.byref(default))
    return total, size.value, default.value, bytes(output)


def synthetic_fonts():
    feat = struct.pack(">IHHI", 0x10000, 4, 0, 0)
    for feature, count, offset, flags, name_id in [
        (0, 1, 60, 0, 260), (1, 1, 64, 0, 256),
        (3, 3, 68, 0x8000, 262), (6, 2, 80, 0xC001, 258),
    ]:
        feat += struct.pack(">HHIHH", feature, count, offset, flags, name_id)
    feat += struct.pack(">14H", 0, 261, 2, 257, 0, 268, 3, 264, 4, 265, 0, 259, 1, 260)
    yield "synthetic AAT features", sfnt("feat", feat)
    fvar = struct.pack(">HHHHHHHH", 1, 0, 16, 2, 2, 20, 0, 12)
    for axis, low, default, high, flags, name_id in [
        ("wght", 100, 400, 900, 0, 256), ("ital", 0, 0, 1, 1, 257),
    ]:
        fvar += struct.pack(">4siiiHH", axis.encode("ascii"), low * 65536,
                            default * 65536, high * 65536, flags, name_id)
    yield "synthetic axes", sfnt("fvar", fvar)
    records = [
        (3, 1, 1033, 256, "Weight".encode("utf-16-be")),
        (3, 10, 1033, 256, "Wide Weight".encode("utf-16-be")),
        (1, 0, 0, 256, b"Mac Weight"),
        (0, 4, 0, 256, "Unicode Weight".encode("utf-16-be")),
        (3, 1, 1036, 256, "Graisse".encode("utf-16-be")),
        (3, 1, 1033, 258, "A😀B".encode("utf-16-be")),
        (1, 0, 0, 257, b"X\x80"),
        (0, 4, 1, 259, "Unicode name".encode("utf-16-be")),
    ]
    name = struct.pack(">HHH", 0, len(records), 6 + 12 * len(records))
    storage = b""
    for platform, encoding, language, name_id, text in sorted(records):
        name += struct.pack(">HHHHHH", platform, encoding, language, name_id, len(text), len(storage))
        storage += text
    ltag = struct.pack(">IIIHHHH", 1, 0, 2, 20, 5, 25, 2) + b"en-USen"
    yield "synthetic localized names", sfnt_tables({"name": name + storage, "ltag": ltag})
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
    list_names = hb.bind("ot_name_list_names", C.POINTER(NameEntry), [P, UP])
    language_text = hb.bind("language_to_string", C.c_char_p, [P])
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
                feature_total = hb.feature_types(hbf, 0, None, None)
                for start, capacity, array, count in [
                    (0, feature_total, True, True), (0, 1, True, True),
                    (1, 3, True, True), (feature_total, 2, True, True),
                    (0xFFFFFFFF, 2, True, True), (0, 99, False, True),
                    (0, 99, True, False), (0, 0, True, True),
                ]:
                    equal((name, "feature_types", start, capacity, array, count),
                          feature_types(hr, hrf, start, capacity, array, count),
                          feature_types(hb, hbf, start, capacity, array, count))
                types = feature_types(hb, hbf, 0, feature_total)[2][:-1]
                for feature in {*types, 0, 3, 37, 38, 0xFFFF, 0x10000, 0xFFFFFFFF}:
                    total = hb.selectors(hbf, feature, 0, None, None, None)
                    for start, capacity, array, count in [
                        (0, total, True, True), (0, 1, True, True),
                        (1, 3, True, True), (total, 2, True, True),
                        (0xFFFFFFFF, 2, True, True), (0, 99, False, True),
                        (0, 99, True, False), (0, 0, True, True),
                    ]:
                        equal((name, "selectors", feature, start, capacity, array, count),
                              selectors(hr, hrf, feature, start, capacity, array, count),
                              selectors(hb, hbf, feature, start, capacity, array, count))
                axis_count = hb.axis_count(hbf)
                equal((name, "axis_count"), hr.axis_count(hrf), axis_count)
                for start, capacity, array, count in [
                    (0, axis_count, True, True), (0, 1, True, True),
                    (1, 3, True, True), (axis_count, 2, True, True),
                    (0xFFFFFFFF, 2, True, True), (0, 99, False, True),
                    (0, 99, True, False), (0, 0, True, True),
                ]:
                    equal((name, "axes", start, capacity, array, count),
                          axes(hr, hrf, start, capacity, array, count),
                          axes(hb, hbf, start, capacity, array, count))
                n = U()
                entries = list_names(hbf, C.byref(n))
                requests = {(entry.name_id, language_text(entry.language))
                            for entry in entries[:n.value]}
                requests.update((name_id, language) for name_id in [1, 2, 256, 0xFFFF]
                                for language in [None, b"en", b"en-us", b"fr", b"fr-fr", b"zz"])
                for name_id, language in sorted(requests, key=lambda entry: (entry[0], entry[1] or b"")):
                    required = name_text(hb, hbf, name_id, language, 0)[0]
                    for capacity in {0, 1, 2, 3, required, required + 1}:
                        equal((name, "name", name_id, language, capacity),
                              name_text(hr, hrf, name_id, language, capacity),
                              name_text(hb, hbf, name_id, language, capacity))
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
