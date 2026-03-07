"""Generate quack-rs logo options using Pillow."""
from PIL import Image, ImageDraw, ImageFont
import math, os

OUT = os.path.dirname(os.path.abspath(__file__))

def try_font(size):
    """Try to load a bold system font, fall back to default."""
    candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSansBold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    ]
    for path in candidates:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()

def try_mono(size):
    candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
        "/usr/share/fonts/truetype/freefont/FreeMonoBold.ttf",
    ]
    for path in candidates:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return try_font(size)

def draw_duck_body(d, cx, cy, scale=1.0, fill="#F5C842", outline="#2A1F00", lw=3):
    """Draw a simple duck: body ellipse, head circle, bill, eye, tail."""
    bw, bh = int(120*scale), int(80*scale)
    hw = int(52*scale)
    # body
    d.ellipse([cx-bw//2, cy-bh//2, cx+bw//2, cy+bh//2], fill=fill, outline=outline, width=lw)
    # head
    hcx, hcy = cx + int(70*scale), cy - int(40*scale)
    d.ellipse([hcx-hw//2, hcy-hw//2, hcx+hw//2, hcy+hw//2], fill=fill, outline=outline, width=lw)
    # bill
    bill_pts = [
        (hcx + int(24*scale), hcy + int(5*scale)),
        (hcx + int(50*scale), hcy + int(0*scale)),
        (hcx + int(24*scale), hcy + int(18*scale)),
    ]
    d.polygon(bill_pts, fill="#FF8C00", outline=outline)
    # eye
    er = int(5*scale)
    d.ellipse([hcx+int(4*scale)-er, hcy-int(10*scale)-er,
               hcx+int(4*scale)+er, hcy-int(10*scale)+er],
              fill="#1A1A1A")
    # tail
    tail_pts = [
        (cx - int(60*scale), cy - int(10*scale)),
        (cx - int(90*scale), cy - int(35*scale)),
        (cx - int(60*scale), cy + int(10*scale)),
    ]
    d.polygon(tail_pts, fill=fill, outline=outline)
    # wing hint
    d.arc([cx-int(30*scale), cy-int(15*scale), cx+int(30*scale), cy+int(35*scale)],
          start=200, end=340, fill=outline, width=lw)

def text_bbox(d, text, font):
    bb = d.textbbox((0, 0), text, font=font)
    return bb[2]-bb[0], bb[3]-bb[1]

# ─── Logo 1: Dark Elegant ───────────────────────────────────────────────────
def logo1():
    W, H = 600, 200
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Dark navy background pill
    d.rounded_rectangle([0, 0, W-1, H-1], radius=30, fill="#0D1117")
    # Duck on left
    draw_duck_body(d, 110, 108, scale=0.85, fill="#F5C842", outline="#0D1117", lw=2)
    # "quack" in yellow, "-rs" in rust orange
    f_big = try_font(72)
    f_sm  = try_font(52)
    d.text((210, 50), "quack", font=f_big, fill="#F5C842")
    tw, _ = text_bbox(d, "quack", f_big)
    d.text((210+tw, 68), "-rs", font=f_sm, fill="#CE422B")
    # tagline
    f_tag = try_font(18)
    d.text((213, 138), "DuckDB extensions in Rust", font=f_tag, fill="#8B9BB4")
    img.save(os.path.join(OUT, "logo1-dark-elegant.png"))
    print("logo1 saved")

# ─── Logo 2: Rust Orange ─────────────────────────────────────────────────────
def logo2():
    W, H = 600, 200
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Warm dark background
    d.rounded_rectangle([0, 0, W-1, H-1], radius=30, fill="#1C0A00")
    # Rust-orange duck
    draw_duck_body(d, 110, 108, scale=0.85, fill="#CE422B", outline="#FF8C00", lw=2)
    f_big = try_font(72)
    f_sm  = try_font(52)
    d.text((210, 50), "quack", font=f_big, fill="#FF8C00")
    tw, _ = text_bbox(d, "quack", f_big)
    d.text((210+tw, 68), "-rs", font=f_sm, fill="#CE422B")
    f_tag = try_font(18)
    d.text((213, 138), "DuckDB extensions in Rust", font=f_tag, fill="#A06040")
    img.save(os.path.join(OUT, "logo2-rust-orange.png"))
    print("logo2 saved")

# ─── Logo 3: Light / Docs-friendly ────────────────────────────────────────────
def logo3():
    W, H = 600, 200
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, W-1, H-1], radius=30, fill="#FAFAFA")
    # Duck outline style (white fill, dark outline)
    draw_duck_body(d, 110, 108, scale=0.85, fill="#FFE066", outline="#1A1A1A", lw=3)
    f_big = try_font(72)
    f_sm  = try_font(52)
    d.text((210, 50), "quack", font=f_big, fill="#1A1A1A")
    tw, _ = text_bbox(d, "quack", f_big)
    d.text((210+tw, 68), "-rs", font=f_sm, fill="#CE422B")
    f_tag = try_font(18)
    d.text((213, 138), "DuckDB extensions in Rust", font=f_tag, fill="#555555")
    img.save(os.path.join(OUT, "logo3-light.png"))
    print("logo3 saved")

# ─── Logo 4: Badge / Icon (square) ────────────────────────────────────────────
def logo4():
    W, H = 300, 300
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Circular badge
    d.ellipse([0, 0, W-1, H-1], fill="#0D1117")
    d.ellipse([6, 6, W-7, H-7], outline="#F5C842", width=4)
    # Centred duck, bigger
    draw_duck_body(d, 148, 145, scale=0.9, fill="#F5C842", outline="#0D1117", lw=2)
    # "Q·RS" monogram underneath
    f = try_font(38)
    label = "Q · RS"
    tw, th = text_bbox(d, label, f)
    d.text(((W-tw)//2, 228), label, font=f, fill="#CE422B")
    img.save(os.path.join(OUT, "logo4-badge.png"))
    print("logo4 saved")

# ─── Logo 5: Gradient-style dark with gear/cog ────────────────────────────────
def logo5():
    W, H = 600, 200
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Two-tone background split
    d.rounded_rectangle([0, 0, W-1, H-1], radius=30, fill="#111827")
    d.rounded_rectangle([0, 0, 195, H-1], radius=30, fill="#1E293B")
    # Cover inner gap of split
    d.rectangle([170, 0, 195, H], fill="#1E293B")
    # Duck
    draw_duck_body(d, 100, 108, scale=0.82, fill="#F5C842", outline="#111827", lw=2)
    # Gear cog (8-tooth) behind the duck as subtle decoration
    cx2, cy2 = 100, 108
    for i in range(8):
        angle = math.radians(i * 45)
        x1 = cx2 + int(55 * math.cos(angle))
        y1 = cy2 + int(55 * math.sin(angle))
        x2 = cx2 + int(65 * math.cos(angle))
        y2 = cy2 + int(65 * math.sin(angle))
        d.line([(x1, y1), (x2, y2)], fill="#2A3A4A", width=10)
    d.ellipse([cx2-48, cy2-48, cx2+48, cy2+48], outline="#2A3A4A", width=4)
    # Redraw duck on top of gear
    draw_duck_body(d, 100, 108, scale=0.82, fill="#F5C842", outline="#111827", lw=2)
    # Monospace text for a "technical" feel
    f_big = try_mono(60)
    f_sm  = try_mono(44)
    d.text((220, 55), "quack", font=f_big, fill="#E2E8F0")
    tw, _ = text_bbox(d, "quack", f_big)
    d.text((220+tw, 71), "-rs", font=f_sm, fill="#CE422B")
    f_tag = try_font(17)
    d.text((222, 140), "DuckDB  ·  Rust  ·  Fast", font=f_tag, fill="#4B6080")
    img.save(os.path.join(OUT, "logo5-technical.png"))
    print("logo5 saved")

logo1()
logo2()
logo3()
logo4()
logo5()
print("All logos generated.")
