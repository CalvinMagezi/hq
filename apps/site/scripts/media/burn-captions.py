# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Burns a WebVTT file into a clip, for machines whose ffmpeg has no subtitle or drawtext filter.
   uv run scripts/media/burn-captions.py in.mp4 captions.vtt out.mp4
Each cue becomes a rounded caption image that ffmpeg's overlay filter shows between its times."""
import re, subprocess, sys, tempfile
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

FONT_CANDIDATES = ["/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"]
WIDTH_SHARE = 0.9
FONT_SHARE = 0.045
BOTTOM_SHARE = 0.17
PAD_SHARE = 0.025


def seconds(stamp: str) -> float:
    parts = stamp.replace(",", ".").split(":")
    parts = [float(p) for p in parts]
    while len(parts) < 3:
        parts.insert(0, 0.0)
    return parts[0] * 3600 + parts[1] * 60 + parts[2]


def cues(vtt: str):
    for block in re.split(r"\n\s*\n", vtt.strip()):
        lines = [l for l in block.splitlines() if l.strip()]
        for i, line in enumerate(lines):
            m = re.match(r"([\d:.,]+)\s*-->\s*([\d:.,]+)", line)
            if m:
                yield seconds(m.group(1)), seconds(m.group(2)), " ".join(lines[i + 1:])


def wrap(draw, text, font, max_width):
    words, lines, cur = text.split(), [], ""
    for w in words:
        trial = f"{cur} {w}".strip()
        if draw.textlength(trial, font=font) <= max_width or not cur:
            cur = trial
        else:
            lines.append(cur)
            cur = w
    return lines + [cur]


def caption_image(text, frame_w, path):
    font_path = next(p for p in FONT_CANDIDATES if Path(p).exists())
    font = ImageFont.truetype(font_path, int(frame_w * FONT_SHARE))
    probe = ImageDraw.Draw(Image.new("RGB", (1, 1)))
    pad = int(frame_w * PAD_SHARE)
    lines = wrap(probe, text, font, frame_w * WIDTH_SHARE - 2 * pad)
    line_h = int(font.size * 1.3)
    w = int(max(probe.textlength(l, font=font) for l in lines)) + 2 * pad
    h = line_h * len(lines) + 2 * pad
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, w, h), radius=pad, fill=(0, 0, 0, 190))
    for i, l in enumerate(lines):
        d.text((w / 2, pad + i * line_h), l, font=font, fill="white", anchor="ma")
    img.save(path)
    return w, h


def main(src, vtt, out):
    probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x", src], capture_output=True, text=True, check=True).stdout.strip()
    fw, fh = map(int, probe.split("x"))
    items = list(cues(Path(vtt).read_text()))
    with tempfile.TemporaryDirectory() as tmp:
        inputs, chain, last = ["-i", src], [], "0:v"
        for n, (start, end, text) in enumerate(items, 1):
            png = f"{tmp}/c{n}.png"
            w, h = caption_image(text, fw, png)
            inputs += ["-i", png]
            y = fh - h - int(fh * BOTTOM_SHARE)
            chain.append(f"[{last}][{n}:v]overlay=x=(W-w)/2:y={y}:enable='between(t,{start},{end})'[v{n}]")
            last = f"v{n}"
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", *inputs, "-filter_complex", ";".join(chain), "-map", f"[{last}]", "-map", "0:a?", "-c:v", "libx264", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "copy", out], check=True)


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
