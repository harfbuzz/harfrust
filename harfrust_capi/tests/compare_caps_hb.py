"""Compare caps and feature queries with HarfBuzz on repository font fixtures.

Usage: python harfrust_capi/tests/compare_caps_hb.py HR_LIBRARY HB_LIBRARY

HarfBuzz enumerates valid scripts, languages, and feature tags for each font;
the same selection, lookup, and pagination queries then run against both APIs.
"""

import ctypes as c
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class Selector(c.Structure):
    _fields_ = [(name, c.c_uint) for name in ("name_id", "enable", "disable", "reserved")]


def bind(path, prefix):
    library = c.CDLL(str(Path(path).resolve()))
    ptr = c.c_void_p
    uint = c.c_uint

    def fn(name, result, *args):
        function = getattr(library, prefix + name)
        function.restype = result
        function.argtypes = args
        return function

    result = {
        "blob": fn("blob_create_from_file", ptr, c.c_char_p),
        "blob_destroy": fn("blob_destroy", None, ptr),
        "face": fn("face_create", ptr, ptr, uint),
        "face_destroy": fn("face_destroy", None, ptr),
        "select": fn("ot_layout_table_select_script", c.c_int, ptr, uint, uint,
                     c.POINTER(uint), c.POINTER(uint), c.POINTER(uint)),
        "find": fn("ot_layout_language_find_feature", c.c_int, ptr, uint, uint,
                   uint, uint, c.POINTER(uint)),
        "tags": fn("ot_layout_table_get_feature_tags", uint, ptr, uint, uint,
                   c.POINTER(uint), c.POINTER(uint)),
        "types": fn("aat_layout_get_feature_types", uint, ptr, uint,
                    c.POINTER(uint), c.POINTER(uint)),
        "selectors": fn("aat_layout_feature_type_get_selector_infos", uint, ptr, uint,
                        uint, c.POINTER(uint), c.POINTER(Selector), c.POINTER(uint)),
    }
    if prefix == "hb_":
        result["scripts"] = fn("ot_layout_table_get_script_tags", uint, ptr, uint,
                               uint, c.POINTER(uint), c.POINTER(uint))
        result["languages"] = fn("ot_layout_script_get_language_tags", uint, ptr, uint,
                                 uint, uint, c.POINTER(uint), c.POINTER(uint))
    return result


def all_tags(function, *args):
    count = c.c_uint(0)
    total = function(*args, 0, c.byref(count), None)
    tags = (c.c_uint * total)()
    count.value = total
    assert function(*args, 0, c.byref(count), tags) == total
    assert count.value == total
    return tuple(tags)


def layout_cases(hb, face):
    cases = {}
    for table in (0x47535542, 0x47504F53):
        scripts = all_tags(hb["scripts"], face, table)
        features = all_tags(hb["tags"], face, table)
        languages = [all_tags(hb["languages"], face, table, index)
                     for index in range(len(scripts))]
        cases[table] = (scripts, languages, features)
    return cases


def query(api, face, cases):
    result = []
    absent = 0x78787878  # xxxx
    for tag, (scripts, languages, features) in cases.items():
        requested_sets = [(), (absent,), (absent,) * 17]
        if scripts:
            requested_sets += [(scripts[-1],), (absent, scripts[-1]),
                               (absent,) * 17 + (scripts[-1],),
                               tuple(reversed(scripts))]
        for requested in requested_sets:
            index, chosen = c.c_uint(99), c.c_uint(99)
            candidates = (c.c_uint * len(requested))(*requested)
            value = api["select"](face, tag, len(requested), candidates,
                                  c.byref(index), c.byref(chosen))
            result.append(("script", tag, requested, value, index.value, chosen.value))
        for script, named_languages in enumerate(languages):
            for language in (0xFFFF, *range(len(named_languages))):
                for feature in (*dict.fromkeys(features), absent):
                    index = c.c_uint(99)
                    value = api["find"](face, tag, script, language, feature,
                                        c.byref(index))
                    result.append(("feature", tag, script, language, feature,
                                   value, index.value))
        total_features = len(features)
        for start, capacity in ((0, 0), (0, total_features), (6, 2),
                                (total_features // 2, 4),
                                (max(total_features - 1, 0), 3),
                                (total_features + 1, 2)):
            count = c.c_uint(capacity)
            tags = (c.c_uint * capacity)()
            total = api["tags"](face, tag, start, c.byref(count), tags)
            result.append(("tags", tag, start, total, count.value, tuple(tags[:count.value])))
    for start, capacity in ((0, 0), (1, 3), (1000, 2)):
        count = c.c_uint(capacity)
        types = (c.c_uint * 3)()
        total = api["types"](face, start, c.byref(count), types)
        result.append(("types", start, total, count.value, tuple(types[:count.value])))
    for feature in (1, 3, 65535):
        for start, capacity in ((0, 0), (1, 2)):
            count, default = c.c_uint(capacity), c.c_uint(99)
            selectors = (Selector * 2)()
            total = api["selectors"](face, feature, start, c.byref(count), selectors,
                                     c.byref(default))
            values = tuple(tuple(getattr(x, name) for name, _ in Selector._fields_)
                           for x in selectors[:count.value])
            result.append(("selectors", feature, start, total, count.value, default.value, values))
    return result


def main():
    hr, hb = bind(sys.argv[1], "hr_"), bind(sys.argv[2], "hb_")
    paths = [
        ROOT / "harfrust/tests/fonts/rb_custom/PT_Sans-Caption-Web-Regular.ttf",
        ROOT / "harfrust/tests/fonts/rb_custom/OpenSans.subset1.ttf",
        ROOT / "harfrust/tests/fonts/rb_custom/NotoSansSinhala.subset1.otf",
        ROOT / "harfrust/tests/fonts/rb_custom/NotoSansMalayalam.subset1.ttf",
        ROOT / "harfrust/tests/fonts/rb_custom/NotoSansMyanmarUI-Regular.subset1.otf",
        ROOT / "harfrust/tests/fonts/rb_custom/Rasa.subset1.otf",
        ROOT / "harfrust/tests/fonts/rb_custom/NotoSansCJK.subset1.otf",
        ROOT / "harfrust/tests/fonts/rb_custom/AdobeBlank-Regular.ttf",
        ROOT / "harfrust_capi/tests/fonts/aat-feat.ttf",
    ]
    for path in paths:
        faces = []
        for api in (hr, hb):
            blob = api["blob"](str(path).encode())
            face = api["face"](blob, 0)
            faces.append((api, blob, face))
        try:
            cases = layout_cases(hb, faces[1][2])
            for table, (scripts, languages, features) in cases.items():
                print(path.name, table.to_bytes(4, "big").decode(),
                      len(scripts), "scripts,", sum(map(len, languages)),
                      "named languages,", len(features), "feature records")
            results = [query(api, face, cases) for api, _, face in faces]
        finally:
            for api, blob, face in faces:
                api["face_destroy"](face)
                api["blob_destroy"](blob)
        for ours, reference in zip(*results):
            if ours != reference:
                print(path.name, "ours:", ours, "HarfBuzz:", reference)
                raise SystemExit(1)
        print(path.name, len(results[0]), "queries match")


if __name__ == "__main__":
    main()
