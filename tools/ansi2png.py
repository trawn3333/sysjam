#!/usr/bin/env python3
"""Render a `tmux capture-pane -p -e` dump (ANSI SGR colours) to a PNG.
usage: ansi2png.py in.ans out.png [cols rows]"""
import os, re, sys, unicodedata
from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont

src, out = sys.argv[1], sys.argv[2]
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
SIZE = 15
font, bold = ImageFont.truetype(FONT, SIZE), ImageFont.truetype(BOLD, SIZE)
# fallbacks for glyphs DejaVu lacks (symbols, monochrome emoji)
FALLBACK = [f for f in ["/usr/share/fonts/truetype/sand-box/google/Noto Sans Symbols/NotoSansSymbols-VariableFont_wght.ttf",
                        "/usr/share/fonts/truetype/libreoffice/opens___.ttf",
                        os.environ.get("SYSJAM_EMOJI_FONT", "/workspace/fonts/NotoEmoji.ttf")] if os.path.exists(f)]
def cmap(path):
    try: return set(TTFont(path, lazy=True).getBestCmap().keys())
    except Exception: return set()
BASE = cmap(FONT)
FB = [(ImageFont.truetype(f, SIZE - 1), cmap(f)) for f in FALLBACK]
def font_for(c, b):
    if ord(c) in BASE: return bold if b else font
    for f, cm in FB:
        if ord(c) in cm: return f
    return bold if b else font
def wide(c):
    return unicodedata.east_asian_width(c) in "WF"
cw = round(font.getlength("M"))
ch = SIZE + 4
BG, FG = (24, 24, 30), (205, 205, 210)
base16 = [(0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16), (36, 114, 200), (188, 63, 188), (17, 168, 205), (204, 204, 204),
          (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67), (59, 142, 234), (214, 112, 214), (41, 184, 219), (240, 240, 240)]
def c256(n):
    if n < 16: return base16[n]
    if n < 232:
        n -= 16; v = [0, 95, 135, 175, 215, 255]
        return (v[n // 36], v[n // 6 % 6], v[n % 6])
    g = 8 + 10 * (n - 232); return (g, g, g)

lines = open(src, encoding="utf-8", errors="replace").read().split("\n")
while lines and not lines[-1].strip(): lines.pop()
cols = int(sys.argv[3]) if len(sys.argv) > 3 else max(len(re.sub(r"\x1b\[[0-9;]*m", "", l)) for l in lines)
rows = int(sys.argv[4]) if len(sys.argv) > 4 else len(lines)
pad = 12
img = Image.new("RGB", (cols * cw + 2 * pad, rows * ch + 2 * pad), BG)
d = ImageDraw.Draw(img)
tok = re.compile(r"\x1b\[([0-9;:]*)m")
fg, bg, b, dim, rev = FG, None, False, False, False
for y, line in enumerate(lines[:rows]):
    # tmux carries SGR state across lines
    x, pos = 0, 0
    for m in list(tok.finditer(line)) + [None]:
        text = line[pos:m.start()] if m else line[pos:]
        for c in text:
            f, g = (bg or BG, fg) if rev else (fg, bg)
            if dim: f = tuple(int(v * 0.6 + BG[i] * 0.4) for i, v in enumerate(f))
            px, py = pad + x * cw, pad + y * ch
            if g: d.rectangle([px, py, px + cw - 1, py + ch - 1], fill=g)
            if c in "▁▂▃▄▅▆▇█":            # draw block elements as exact rectangles
                h = ("▁▂▃▄▅▆▇█".index(c) + 1) / 8
                d.rectangle([px, py + int(ch * (1 - h)), px + cw - 1, py + ch - 1], fill=f)
            elif c in "▏▎▍▌▋▊▉":
                w = ("▏▎▍▌▋▊▉".index(c) + 1) / 8
                d.rectangle([px, py, px + max(1, int(cw * w)) - 1, py + ch - 1], fill=f)
            elif c in "─│┌┐└┘":
                mx, my = px + cw // 2, py + ch // 2
                if c in "─┌└┐┘":
                    x0 = mx if c in "┌└" else px; x1 = mx if c in "┐┘" else px + cw - 1
                    d.line([x0, my, x1, my], fill=f)
                if c in "│┌┐└┘":
                    y0 = my if c in "┌┐" else py; y1 = my if c in "└┘" else py + ch - 1
                    d.line([mx, y0, mx, y1], fill=f)
            elif c != " ":
                if wide(c) and g: d.rectangle([px + cw, py, px + 2 * cw - 1, py + ch - 1], fill=g)
                d.text((px, py + 1), c, font=font_for(c, b), fill=f)
            x += 2 if wide(c) else 1
        if not m: break
        pos = m.end()
        p = [int(v) if v else 0 for v in re.split("[;:]", m.group(1))] if m.group(1) else [0]
        i = 0
        while i < len(p):
            v = p[i]
            if v == 0: fg, bg, b, dim, rev = FG, None, False, False, False
            elif v == 1: b = True
            elif v == 2: dim = True
            elif v == 7: rev = True
            elif v == 22: b = dim = False
            elif v == 27: rev = False
            elif 30 <= v <= 37: fg = base16[v - 30]
            elif 90 <= v <= 97: fg = base16[v - 82]
            elif 40 <= v <= 47: bg = base16[v - 40]
            elif 100 <= v <= 107: bg = base16[v - 92]
            elif v == 39: fg = FG
            elif v == 49: bg = None
            elif v in (38, 48):
                if p[i + 1] == 5: col = c256(p[i + 2]); i += 2
                elif ":2::" in m.group(1): col = tuple(p[i + 3:i + 6]); i += 5
                else: col = tuple(p[i + 2:i + 5]); i += 4
                if v == 38: fg = col
                else: bg = col
            i += 1
img.save(out)
print(f"{out}: {img.size[0]}x{img.size[1]}")
