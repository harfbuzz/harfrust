#!/usr/bin/env python3
"""Compare the C API's layout metadata queries with a HarfBuzz shared library.

Uses only Python's standard library. Build harfrust_capi first, then run:
  python3 scripts/test-capi-ot-queries.py --library target/debug/libharfrust_c.so \
      --harfbuzz /path/to/libharfbuzz.so --fonts harfrust/tests/fonts
"""
import argparse
import ctypes as C
from pathlib import Path

U = C.c_uint
P = C.c_void_p
UP = C.POINTER(U)


def bind(library, prefix, name, result, args):
    function = getattr(library, prefix + name)
    function.restype = result
    function.argtypes = args
    return function


class Api:
    def __init__(self, library, prefix):
        self.blob_create = bind(library, prefix, "blob_create", P,
                                [P, U, C.c_int, P, P])
        self.blob_destroy = bind(library, prefix, "blob_destroy", None, [P])
        self.face_create = bind(library, prefix, "face_create", P, [P, U])
        self.face_count = bind(library, prefix, "face_count", U, [P])
        self.face_destroy = bind(library, prefix, "face_destroy", None, [P])
        self.select = bind(library, prefix, "ot_layout_table_select_script", C.c_int,
                           [P, U, U, UP, UP, UP])
        self.features = bind(library, prefix, "ot_layout_table_get_feature_tags", U,
                             [P, U, U, UP, UP])
        self.find = bind(library, prefix, "ot_layout_language_find_feature", C.c_int,
                         [P, U, U, U, U, UP])

    def face(self, data, index):
        blob = self.blob_create(data, len(data), 0, None, None)
        face = self.face_create(blob, index)
        self.blob_destroy(blob)
        return face


def tag(text):
    return int.from_bytes(text.encode("ascii"), "big")


def select(api, face, table, scripts):
    index, chosen = U(123), U(123)
    result = api.select(face, table, len(scripts), (U * len(scripts))(*scripts),
                        C.byref(index), C.byref(chosen))
    return result, index.value, chosen.value


def page(api, face, table, start, capacity, array=True, count=True):
    size = U(capacity)
    output = (U * (capacity + 1))(*([0xDEADBEEF] * (capacity + 1)))
    total = api.features(face, table, start, C.byref(size) if count else None,
                         output if array else None)
    return total, size.value, list(output)


def find(api, face, table, script, language, feature):
    index = U(123)
    result = api.find(face, table, script, language, feature, C.byref(index))
    return result, index.value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", required=True)
    parser.add_argument("--harfbuzz", required=True, help="HarfBuzz 14.5.1 or newer shared library")
    parser.add_argument("--fonts", type=Path, required=True)
    args = parser.parse_args()
    hr = Api(C.CDLL(args.library), "hr_")
    hb_lib = C.CDLL(args.harfbuzz)
    hb = Api(hb_lib, "hb_")
    version = bind(hb_lib, "hb_", "version_atleast", C.c_int, [U, U, U])
    if not version(14, 5, 1):
        parser.error("use HarfBuzz >= 14.5.1 for nullable pagination tests")
    has_sub = bind(hb_lib, "hb_", "ot_layout_has_substitution", C.c_int, [P])
    has_pos = bind(hb_lib, "hb_", "ot_layout_has_positioning", C.c_int, [P])
    scripts = bind(hb_lib, "hb_", "ot_layout_table_get_script_tags", U,
                   [P, U, U, UP, UP])
    languages = bind(hb_lib, "hb_", "ot_layout_script_get_language_tags", U,
                     [P, U, U, U, UP, UP])
    checks = faces = skipped = 0

    def equal(label, actual, expected):
        nonlocal checks
        checks += 1
        assert actual == expected, (label, actual, expected)

    for path in sorted(args.fonts.rglob("*")):
        if path.suffix.lower() not in {".ttf", ".otf", ".ttc"}:
            continue
        data = path.read_bytes()
        blob = hb.blob_create(data, len(data), 0, None, None)
        count = hb.face_count(blob)
        hb.blob_destroy(blob)
        for index in range(count):
            hrf, hbf = hr.face(data, index), hb.face(data, index)
            faces += 1
            try:
                for table in [tag("GSUB"), tag("GPOS"), 0]:
                    # Fontations queries readable metadata directly; HarfBuzz
                    # sanitizes the entire layout table, including its lookups.
                    # The shaping corpus contains deliberately broken tables.
                    if (table == tag("GSUB") and not has_sub(hbf)) or (
                            table == tag("GPOS") and not has_pos(hbf)):
                        skipped += 1
                        continue
                    total = scripts(hbf, table, 0, None, None)
                    values = (U * total)()
                    n = U(total)
                    scripts(hbf, table, 0, C.byref(n), values)
                    for requested in [[], [tag("zzzz")], [tag("latn")],
                                      [tag("arab"), tag("latn")], list(values),
                                      list(reversed(values))]:
                        actual = select(hr, hrf, table, requested)
                        expected = select(hb, hbf, table, requested)
                        # Duplicate script records are outside the specification;
                        # either matching index is acceptable for that tag.
                        if list(values).count(expected[2]) > 1:
                            equal((str(path), "select duplicate", table, requested),
                                  (actual[0], actual[2]), (expected[0], expected[2]))
                        else:
                            equal((str(path), "select", table, requested), actual, expected)
                    feature_total = hb.features(hbf, table, 0, None, None)
                    for start, capacity, array, count in [
                        (0, feature_total, True, True), (0, 1, True, True),
                        (1, 3, True, True), (feature_total, 2, True, True),
                        (0xFFFFFFFF, 2, True, True), (0, 99, False, True),
                        (0, 99, True, False), (0, 0, True, True),
                    ]:
                        equal((str(path), "page", table, start, capacity, array, count),
                              page(hr, hrf, table, start, capacity, array, count),
                              page(hb, hbf, table, start, capacity, array, count))
                    feature_tags = page(hb, hbf, table, 0, feature_total)[2][:-1]
                    for script in [*range(total), 0xFFFF, 0xFFFFFFFF]:
                        language_count = languages(hbf, table, script, 0, None, None)
                        for language in [*range(language_count), 0xFFFF, 0xFFFFFFFF]:
                            for feature in {*feature_tags, tag("zzzz"), 0}:
                                equal((str(path), "find", table, script, language, feature),
                                      find(hr, hrf, table, script, language, feature),
                                      find(hb, hbf, table, script, language, feature))
            finally:
                hr.face_destroy(hrf)
                hb.face_destroy(hbf)
    print(f"Matched HarfBuzz in {checks} query comparisons across {faces} font faces "
          f"({skipped} absent or HarfBuzz-rejected layout tables skipped).")


if __name__ == "__main__":
    main()
