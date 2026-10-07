#!/usr/bin/env python3
"""Extract HarfBuzz's reverse OpenType language map into Rust.

Usage: python scripts/gen-tag-reverse.py path/to/hb-ot-tag-table.hh
"""

import re
import sys
from pathlib import Path

source = Path(sys.argv[1]).read_text(encoding="utf-8")
hb_tag = re.compile(r"HB_TAG\('([^']{1})','([^']{1})','([^']{1})','([^']{1})'\)")
reverse = {}
for table in ("ot_languages2", "ot_languages3"):
    section = source.split(f"static const LangTag {table}[] = {{", 1)[1].split("};", 1)[0]
    for line in section.splitlines():
        if not line.lstrip().startswith("{HB_TAG("):
            continue
        tags = hb_tag.findall(line)
        if len(tags) == 2:
            language = "".join(tags[0]).rstrip()
            tag = "".join(tags[1])
            reverse.setdefault(tag, language)

section = source.split("hb_ot_ambiguous_tag_to_language (hb_tag_t tag)", 1)[1]
section = section.split("  default:", 1)[0]
ambiguous = re.findall(
    r"case HB_TAG\('([^']{1})','([^']{1})','([^']{1})','([^']{1})'\):"
    r"[^\n]*\n\s*return hb_language_from_string \(\"([^\"]+)\", -1\);",
    section,
)
for a, b, c, d, language in ambiguous:
    reverse[a + b + c + d] = language
if len(reverse) < 100 or not ambiguous:
    raise SystemExit("could not extract reverse language map")

target = Path(__file__).resolve().parents[1] / "harfrust/src/tag_reverse.rs"
with target.open("w", encoding="utf-8", newline="\n") as output:
    output.write("// Generated from HarfBuzz's hb-ot-tag-table.hh by scripts/gen-tag-reverse.py.\n")
    output.write("// Regenerate alongside tag_table.rs when updating HarfBuzz's language data.\n\n")
    output.write("pub(crate) static OT_LANGUAGES_REVERSE: &[([u8; 4], &str)] = &[\n")
    for tag, language in sorted(reverse.items()):
        output.write(f'    (*b"{tag}", "{language}"),\n')
    output.write("];\n")
print(f"wrote {target} ({len(reverse)} entries)")
