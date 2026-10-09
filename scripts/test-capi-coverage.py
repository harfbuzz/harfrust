#!/usr/bin/env python3
"""Compare nominal cmap coverage with HarfBuzz.

Build harfrust_capi, then run:
  python3 scripts/test-capi-coverage.py --library /path/to/libharfrust_c.so \
      --harfbuzz /path/to/libharfbuzz.so --fonts harfrust/tests/fonts

HarfBuzz 14.5.1 incorrectly truncates format-13 constant-glyph ranges as though
glyph IDs incremented within each range. For those subtables, independently
check HarfRust's complete ranges and report the HarfBuzz difference separately.
"""
import argparse
import ctypes as C
from pathlib import Path
import struct

U, P = C.c_uint, C.c_void_p
SEED = 0x10FFFF


class Api:
    def __init__(self, library, prefix):
        self.library = C.CDLL(library)

        def bind(name, result, arguments):
            function = getattr(self.library, prefix + name)
            function.restype = result
            function.argtypes = arguments
            return function

        self.blob = bind("blob_create", P, [P, U, C.c_int, P, P])
        self.drop_blob = bind("blob_destroy", None, [P])
        self.blob_data = bind("blob_get_data", P, [P, C.POINTER(U)])
        self.face = bind("face_create", P, [P, U])
        self.face_count = bind("face_count", U, [P])
        self.drop_face = bind("face_destroy", None, [P])
        self.table = bind("face_reference_table", P, [P, U])
        self.glyph_count = bind("face_get_glyph_count", U, [P])
        self.make_set = bind("set_create", P, [])
        self.drop_set = bind("set_destroy", None, [P])
        self.add = bind("set_add", None, [P, U])
        self.collect = bind("face_collect_unicodes", None, [P, P])
        self.next = bind("set_next", C.c_int, [P, C.POINTER(U)])

    def coverage(self, face):
        output = self.make_set()
        self.add(output, SEED)
        self.collect(face, output)
        value, values = U(0xFFFFFFFF), set()
        while self.next(output, C.byref(value)):
            values.add(value.value)
        self.drop_set(output)
        return values

    def format13_coverage(self, face):
        blob = self.table(face, int.from_bytes(b"cmap", "big"))
        size = U()
        pointer = self.blob_data(blob, C.byref(size))
        data = C.string_at(pointer, size.value) if size.value else b""
        self.drop_blob(blob)
        if len(data) < 4:
            return None
        count = struct.unpack_from(">H", data, 2)[0]
        records = [struct.unpack_from(">HHI", data, 4 + i * 8) for i in range(count)]
        priority = [(3, 0), (3, 10), (0, 6), (0, 4), (3, 1),
                    (0, 3), (0, 2), (0, 1), (0, 0), (1, 0)]
        for platform, encoding in priority:
            for record_platform, record_encoding, offset in records:
                if (platform, encoding) != (record_platform, record_encoding):
                    continue
                format_id = struct.unpack_from(">H", data, offset)[0]
                if format_id not in (0, 4, 6, 10, 12, 13):
                    continue
                if format_id != 13:
                    return None
                groups = struct.unpack_from(">I", data, offset + 12)[0]
                glyph_count = self.glyph_count(face)
                result = {SEED}
                for i in range(groups):
                    first, last, glyph = struct.unpack_from(">III", data, offset + 16 + i * 12)
                    if 0 < glyph < glyph_count:
                        result.update(range(first, min(last, 0x10FFFF) + 1))
                return result
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True)
    parser.add_argument("--harfbuzz", required=True)
    parser.add_argument("--fonts", type=Path, required=True)
    args = parser.parse_args()
    hr, hb = Api(args.library, "hr_"), Api(args.harfbuzz, "hb_")
    matched, corrected = 0, 0
    for path in sorted(args.fonts.rglob("*")):
        if path.suffix.lower() not in {".ttf", ".otf", ".ttc"}:
            continue
        data = path.read_bytes()
        hr_blob = hr.blob(data, len(data), 0, None, None)
        hb_blob = hb.blob(data, len(data), 0, None, None)
        for index in range(hb.face_count(hb_blob)):
            hr_face, hb_face = hr.face(hr_blob, index), hb.face(hb_blob, index)
            try:
                actual, expected = hr.coverage(hr_face), hb.coverage(hb_face)
                if actual == expected:
                    matched += 1
                else:
                    complete = hr.format13_coverage(hr_face)
                    assert complete is not None and actual == complete and expected <= actual, (
                        path, index, len(actual), len(expected), sorted(actual ^ expected)[:10])
                    print(f"Complete format-13 coverage: {path} face {index}; "
                          f"HarfRust {len(actual)} entries, HarfBuzz {len(expected)}")
                    corrected += 1
            finally:
                hr.drop_face(hr_face)
                hb.drop_face(hb_face)
        hr.drop_blob(hr_blob)
        hb.drop_blob(hb_blob)
    print(f"Matched {matched} coverage sets; independently checked {corrected} "
          "complete format-13 coverage sets (counts include a seeded existing entry).")


if __name__ == "__main__":
    main()
