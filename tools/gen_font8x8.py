#!/usr/bin/env python3
"""Generate kernel/src/font8x8.rs from the public-domain font8x8_basic table.

TempleOS-RS Phase 2 needs an 8x8 glyph source for the text overlay that keeps
the console readable after switching to VGA mode 12h (SPEC_TOS BC2). Reading the
font out of VGA plane 2 is the classic trick, but it depends on what the firmware
happened to leave there; a table compiled into the kernel is deterministic and
lets a screendump assertion mean something.

Provenance: Daniel Hepper's font8x8 (Public Domain), itself based on Marcel
Sondaar / IBM public-domain VGA fonts. This script exists so the table is
*reproducible* rather than hand-transcribed; run it only to regenerate.

Usage: python tools/gen_font8x8.py          (writes kernel/src/font8x8.rs)
"""
import os
import re
import sys
import urllib.request

SRC_URL = "https://raw.githubusercontent.com/dhepper/font8x8/master/font8x8_basic.h"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "kernel", "src", "font8x8.rs")

HEADER = '''//! 8x8 bitmap font for the VGA graphics console (TempleOS-RS Phase 2).
//!
//! Character codes 0x00-0x7F, eight bytes each, MSB = leftmost pixel.
//! **Generated file** — do not edit by hand; regenerate with
//! `python3 tools/gen_font8x8.py`.
//!
//! Provenance: Daniel Hepper's `font8x8` (Public Domain), based on Marcel
//! Sondaar / IBM public-domain VGA fonts. Compiled in rather than read from
//! VGA plane 2 so glyph rendering does not depend on firmware state.

/// 128 glyphs, 8 rows each. bit 0 of a row is its LEFTmost pixel: that is
/// the source table's convention (font8x8_basic), and gfx::glyph reads it.
pub static FONT8X8: [[u8; 8]; 128] = [
'''


def main() -> int:
    with urllib.request.urlopen(SRC_URL, timeout=30) as r:
        text = r.read().decode("utf-8", errors="replace")

    rows = []
    for line in text.splitlines():
        m = re.match(r"\s*\{\s*((?:0[xX][0-9a-fA-F]{1,2}\s*,\s*){7}0[xX][0-9a-fA-F]{1,2})\s*\}\s*,?\s*(?://.*)?$", line)
        if not m:
            continue
        vals = [int(v.strip(), 16) for v in m.group(1).split(",")]
        if len(vals) == 8:
            rows.append(vals)

    if len(rows) != 128:
        print(f"ERROR: parsed {len(rows)} glyphs, expected 128", file=sys.stderr)
        return 1

    # Sanity: a real font must have the letter 'A' (0x41) and '0' (0x30) drawn.
    for code, name in ((0x41, "A"), (0x30, "0"), (0x20, "space")):
        if code != 0x20 and all(b == 0 for b in rows[code]):
            print(f"ERROR: glyph for {name!r} is blank — parse went wrong", file=sys.stderr)
            return 1
    if any(b != 0 for b in rows[0x20]):
        print("ERROR: space glyph should be blank", file=sys.stderr)
        return 1

    out = [HEADER]
    for i, v in enumerate(rows):
        ch = chr(i) if 0x20 <= i < 0x7F else " "
        out.append("    [{}], // 0x{:02X} {!r}\n".format(
            ", ".join("0x{:02X}".format(b) for b in v), i, ch))
    out.append("];\n")

    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("".join(out))
    print(f"wrote {OUT} ({len(rows)} glyphs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
