"""
quack-rs logos — v3: pixel-perfect, polished, consistent

Design system
─────────────
Duck      : radial 3-stop gold gradient + specular highlights + proper wing
            (no more belly-frown stroke; wing is a filled leaf shape)
Tail      : taller, narrower curl — clearly a tail, not a bump
Beak      : clean rounded leaf; bill_color uniform across logos
Typography: Liberation Sans (installed); weight 700 (Bold) is the max present
Palette   : strict constants — no per-logo colour drift
Shadows   : tuned per context (dark bg = stronger; light bg = warm, softer)
"""
import cairosvg, os

OUT = os.path.dirname(os.path.abspath(__file__))

# ── Palette ────────────────────────────────────────────────────────────────────
GOLD_HI   = "#FFF2A8"   # gradient highlight (upper-right light hit)
GOLD_MID  = "#F5C420"   # rich gold midtone
GOLD_SHA  = "#A87200"   # deep amber shadow
BEAK      = "#E06818"   # beak
RUST      = "#CE422B"   # -rs accent  ← consistent across ALL logos
DARK1     = "#0D1117"   # darkest bg stop
DARK2     = "#161B22"   # lighter dark bg stop
TEXT_LT   = "#E6EDF3"   # wordmark on dark bg
TEXT_DK   = "#0D1117"   # wordmark on light bg
TAG_DK    = "#526880"   # tagline on dark bg
TAG_LT    = "#6B7280"   # tagline on light bg
FONT      = "'Liberation Sans', Arial, sans-serif"


# ── Duck builder ───────────────────────────────────────────────────────────────

def duck(cx, cy, s=1.0,
         dark_color=GOLD_SHA,
         bill_color=BEAK,
         eye_color=DARK1,
         grad_id="dg",
         shadow_filter="url(#ds)"):
    """
    Polished rubber duck facing right.
    cx, cy = body-ellipse centre.  s = uniform scale.
    Improvements over v2:
      - Tail: taller, narrower, clearly a tail curl
      - Wing: filled leaf on upper body (replaces belly-frown stroke)
      - Specular: subtle white ellipses on head & body for 3-D sheen
    """
    bx, by = cx, cy
    rx, ry = 68 * s, 46 * s
    hx, hy = cx + 52 * s, cy - 42 * s
    hr = 32 * s

    # ── Tail: tall upward curl, narrow at tip ──────────────────────────────────
    tx0 = cx - rx
    tail = (
        f"M {tx0:.2f},{by + 2*s:.2f} "
        f"C {cx - 86*s:.2f},{by - 14*s:.2f} "
        f"  {cx - 94*s:.2f},{by - 50*s:.2f} "
        f"  {cx - 78*s:.2f},{by - 66*s:.2f} "
        f"C {cx - 64*s:.2f},{by - 76*s:.2f} "
        f"  {cx - 50*s:.2f},{by - 58*s:.2f} "
        f"  {cx - 54*s:.2f},{by - 30*s:.2f} "
        f"C {cx - 58*s:.2f},{by - 12*s:.2f} "
        f"  {cx - 64*s:.2f},{by - 3*s:.2f} "
        f"  {tx0:.2f},{by + 2*s:.2f} Z"
    )

    # ── Bill: smooth rounded leaf pointing right ───────────────────────────────
    b1x, b1y = hx + 24 * s, hy - 10 * s
    b2x, b2y = hx + 62 * s, hy + 1 * s
    b3x, b3y = hx + 24 * s, hy + 14 * s
    bill = (
        f"M {b1x:.2f},{b1y:.2f} "
        f"Q {b2x + 4*s:.2f},{b1y:.2f} {b2x:.2f},{b2y:.2f} "
        f"Q {b2x + 4*s:.2f},{b3y:.2f} {b3x:.2f},{b3y:.2f} "
        f"Q {hx + 18*s:.2f},{hy + 2*s:.2f} {b1x:.2f},{b1y:.2f} Z"
    )

    # ── Eye ────────────────────────────────────────────────────────────────────
    ex, ey, er = hx + 5 * s, hy - 10 * s, 6.5 * s

    # ── Wing: filled leaf on upper body (NOT a belly stroke) ──────────────────
    # Upper arc peaks at ~cy-28s; lower arc is flatter → teardrop/leaf
    wx0, wy0 = cx - 8*s,  cy - 2*s
    wxu, wyu = cx + 18*s, cy - 28*s   # upper arc control
    wx2, wy2 = cx + 50*s, cy - 2*s
    wxl, wyl = cx + 18*s, cy - 10*s   # lower arc control (flatter)
    wing = (
        f"M {wx0:.2f},{wy0:.2f} "
        f"Q {wxu:.2f},{wyu:.2f} {wx2:.2f},{wy2:.2f} "
        f"Q {wxl:.2f},{wyl:.2f} {wx0:.2f},{wy0:.2f} Z"
    )

    # ── Specular highlights (3-D sheen) ───────────────────────────────────────
    # Head: upper-left highlight (where light hits a sphere first)
    sh_hx = hx - 10 * s
    sh_hy = hy - 14 * s
    # Body: broad diffuse highlight on upper face
    sh_bx = bx + 8 * s
    sh_by = by - 26 * s

    fill_ref = f"url(#{grad_id})"

    return f"""
  <g filter="{shadow_filter}">
    <path d="{tail}" fill="{fill_ref}" stroke="none"/>
    <ellipse cx="{bx:.2f}" cy="{by:.2f}" rx="{rx:.2f}" ry="{ry:.2f}"
             transform="rotate(-4,{bx:.2f},{by:.2f})"
             fill="{fill_ref}" stroke="none"/>
    <circle  cx="{hx:.2f}" cy="{hy:.2f}" r="{hr:.2f}"
             fill="{fill_ref}" stroke="none"/>
    <path d="{wing}" fill="{dark_color}" opacity="0.28" stroke="none"/>
    <path d="{bill}" fill="{bill_color}" stroke="none"/>
    <circle  cx="{ex:.2f}" cy="{ey:.2f}" r="{er:.2f}"
             fill="{eye_color}" stroke="none"/>
    <circle  cx="{ex + 2.5*s:.2f}" cy="{ey - 2.8*s:.2f}" r="{2.5*s:.2f}"
             fill="white" stroke="none"/>
    <ellipse cx="{sh_hx:.2f}" cy="{sh_hy:.2f}"
             rx="{9*s:.2f}" ry="{6*s:.2f}"
             transform="rotate(-30,{sh_hx:.2f},{sh_hy:.2f})"
             fill="white" opacity="0.18" stroke="none"/>
    <ellipse cx="{sh_bx:.2f}" cy="{sh_by:.2f}"
             rx="{16*s:.2f}" ry="{9*s:.2f}"
             transform="rotate(-20,{sh_bx:.2f},{sh_by:.2f})"
             fill="white" opacity="0.10" stroke="none"/>
  </g>"""


# ── Logo 1: Dark Elegant — primary README horizontal lockup ───────────────────

def logo1():
    W, H = 660, 210

    # Duck body at (108, 108) @ s=0.92
    # Tail left edge ≈ x 23 | Beak tip ≈ x 217 | Text from x 228
    # Text block: cap-top ≈ 65, tagline baseline ≈ 151 → visual centre ≈ 108 ✓

    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="dg" cx="62%" cy="24%" r="68%">
      <stop offset="0%"   stop-color="{GOLD_HI}"/>
      <stop offset="38%"  stop-color="{GOLD_MID}"/>
      <stop offset="100%" stop-color="{GOLD_SHA}"/>
    </radialGradient>
    <filter id="ds" x="-28%" y="-28%" width="180%" height="180%">
      <feDropShadow dx="0" dy="6" stdDeviation="12"
                    flood-color="#000000" flood-opacity="0.55"/>
    </filter>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%"   stop-color="#0F1419"/>
      <stop offset="100%" stop-color="{DARK2}"/>
    </linearGradient>
  </defs>

  <rect width="{W}" height="{H}" rx="22" fill="url(#bg)"/>

  {duck(108, 108, s=0.92)}

  <!-- Wordmark -->
  <text x="228" y="122"
        font-family="{FONT}" font-size="80" font-weight="700"
        letter-spacing="-2">
    <tspan fill="{TEXT_LT}">quack</tspan><tspan fill="{RUST}">-rs</tspan>
  </text>

  <!-- Tagline: left-aligned to wordmark, refined spacing -->
  <text x="230" y="151"
        font-family="{FONT}" font-size="16" font-weight="400"
        letter-spacing="1.8" fill="{TAG_DK}">DUCKDB · RUST · FAST</text>
</svg>"""
    _save(svg, "logo1-dark-elegant")


# ── Logo 2: Light / Docs ───────────────────────────────────────────────────────

def logo2():
    W, H = 660, 210

    # Duck body at (100, 108) @ s=0.88 — slightly smaller for better text ratio
    # Beak tip ≈ x 203 | Text from x 214 → 11 px clear gap
    # Warm amber shadow on white for subtle depth

    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="dg" cx="62%" cy="24%" r="68%">
      <stop offset="0%"   stop-color="{GOLD_HI}"/>
      <stop offset="38%"  stop-color="{GOLD_MID}"/>
      <stop offset="100%" stop-color="{GOLD_SHA}"/>
    </radialGradient>
    <filter id="ds" x="-28%" y="-28%" width="180%" height="180%">
      <feDropShadow dx="0" dy="4" stdDeviation="9"
                    flood-color="#7A5500" flood-opacity="0.22"/>
    </filter>
  </defs>

  <!-- White card with subtle border -->
  <rect width="{W}" height="{H}" rx="18" fill="#FFFFFF"/>
  <rect width="{W}" height="{H}" rx="18" fill="none"
        stroke="#DDE3EA" stroke-width="1.5"/>

  {duck(100, 108, s=0.88, eye_color="#1A1A1A")}

  <!-- Wordmark -->
  <text x="214" y="122"
        font-family="{FONT}" font-size="80" font-weight="700"
        letter-spacing="-2">
    <tspan fill="{TEXT_DK}">quack</tspan><tspan fill="{RUST}">-rs</tspan>
  </text>

  <!-- Tagline: same left edge as wordmark -->
  <text x="216" y="151"
        font-family="{FONT}" font-size="16" font-weight="400"
        letter-spacing="0.4" fill="{TAG_LT}">DuckDB extensions in Rust</text>
</svg>"""
    _save(svg, "logo2-light-docs")


# ── Logo 3: Circular Badge Icon ────────────────────────────────────────────────

def logo3():
    W = H = 360
    cx, cy = W // 2, H // 2   # 180, 180

    # Duck body at (172, 148) @ s=0.93 — fills the upper half of the badge
    # Water ripples under duck bridge the gap to the label
    # Text at y=314, font-size=40 → cap top ≈ 285; ripples end ≈ 236 → good gap

    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="bgr" cx="38%" cy="34%" r="68%">
      <stop offset="0%"   stop-color="#1C2330"/>
      <stop offset="100%" stop-color="#080C14"/>
    </radialGradient>
    <radialGradient id="dg" cx="62%" cy="24%" r="68%">
      <stop offset="0%"   stop-color="{GOLD_HI}"/>
      <stop offset="38%"  stop-color="{GOLD_MID}"/>
      <stop offset="100%" stop-color="{GOLD_SHA}"/>
    </radialGradient>
    <!-- Double ring: outer gold, inner rust — refined border detail -->
    <linearGradient id="ring1" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%"   stop-color="#F5C840"/>
      <stop offset="100%" stop-color="#D4A020"/>
    </linearGradient>
    <filter id="ds" x="-30%" y="-30%" width="180%" height="180%">
      <feDropShadow dx="0" dy="6" stdDeviation="12"
                    flood-color="#000" flood-opacity="0.60"/>
    </filter>
    <clipPath id="circ">
      <circle cx="{cx}" cy="{cy}" r="{cx - 1}"/>
    </clipPath>
  </defs>

  <!-- Background disc -->
  <circle cx="{cx}" cy="{cy}" r="{cx}" fill="url(#bgr)"/>

  <!-- Outer ring (gold gradient) -->
  <circle cx="{cx}" cy="{cy}" r="{cx - 5}"
          fill="none" stroke="url(#ring1)" stroke-width="4.5"/>
  <!-- Inner accent ring (rust, thinner) -->
  <circle cx="{cx}" cy="{cy}" r="{cx - 12}"
          fill="none" stroke="{RUST}" stroke-width="1.5" opacity="0.5"/>

  <!-- Duck clipped to circle -->
  <g clip-path="url(#circ)">
    {duck(172, 148, s=0.93)}
  </g>

  <!-- Water ripples (bridge between duck and label) -->
  <ellipse cx="{cx - 8}" cy="210" rx="72" ry="7.5"
           fill="none" stroke="#2A4060" stroke-width="2.0" opacity="0.70"/>
  <ellipse cx="{cx - 8}" cy="221" rx="50" ry="5.5"
           fill="none" stroke="#2A4060" stroke-width="1.4" opacity="0.48"/>
  <ellipse cx="{cx - 8}" cy="230" rx="30" ry="3.5"
           fill="none" stroke="#2A4060" stroke-width="0.9" opacity="0.28"/>

  <!-- Label: two tspan trick centred in badge -->
  <!-- "quack" anchor=end / "-rs" anchor=start at the same split x=203 -->
  <text x="203" y="314"
        font-family="{FONT}" font-size="40" font-weight="700"
        letter-spacing="-0.5" text-anchor="end"
        fill="{TEXT_LT}">quack</text>
  <text x="203" y="314"
        font-family="{FONT}" font-size="40" font-weight="700"
        letter-spacing="-0.5" text-anchor="start"
        fill="{RUST}">-rs</text>
</svg>"""
    _save(svg, "logo3-badge-icon")


# ── Logo 4: Rust Ember — warm gradient wash, no harsh panel split ─────────────

def logo4():
    W, H = 660, 210

    # Design intent: evoke Rust-language warmth without a jarring red block.
    # A deep ember gradient bleeds in from the left — dark ochre-rust at the
    # edge, dissolving into the same charcoal used by the other dark logos.
    # The duck sits in the warm zone; wordmark and tagline on the cool side.

    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <!-- Duck gold — slightly warmer/more orange to harmonise with the ember bg -->
    <radialGradient id="dg" cx="62%" cy="24%" r="68%">
      <stop offset="0%"   stop-color="#FFF4B0"/>
      <stop offset="38%"  stop-color="#F0B818"/>
      <stop offset="100%" stop-color="#9E6A00"/>
    </radialGradient>
    <filter id="ds" x="-25%" y="-25%" width="175%" height="175%">
      <feDropShadow dx="0" dy="5" stdDeviation="10"
                    flood-color="#2A0800" flood-opacity="0.55"/>
    </filter>
    <!-- Background: deep charcoal with a warm ember wash on the left -->
    <linearGradient id="bg4" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%"   stop-color="#2A1208"/>
      <stop offset="42%"  stop-color="#1A100A"/>
      <stop offset="100%" stop-color="#111622"/>
    </linearGradient>
    <!-- Ember radial: extra warmth pooled where the duck lives -->
    <radialGradient id="ember" cx="24%" cy="50%" r="44%">
      <stop offset="0%"   stop-color="#7A2C10" stop-opacity="0.55"/>
      <stop offset="100%" stop-color="#7A2C10" stop-opacity="0"/>
    </radialGradient>
    <clipPath id="full">
      <rect width="{W}" height="{H}" rx="22"/>
    </clipPath>
  </defs>

  <g clip-path="url(#full)">
    <!-- Dark background with horizontal warm fade -->
    <rect width="{W}" height="{H}" fill="url(#bg4)"/>
    <!-- Radial ember pool behind the duck -->
    <rect width="{W}" height="{H}" fill="url(#ember)"/>
  </g>

  {duck(110, 108, s=0.92,
        dark_color="#9E6A00",
        bill_color="#E06418")}

  <!-- Wordmark -->
  <text x="234" y="122"
        font-family="{FONT}" font-size="80" font-weight="700"
        letter-spacing="-2">
    <tspan fill="{TEXT_LT}">quack</tspan><tspan fill="#D4563C">-rs</tspan>
  </text>

  <!-- Tagline — warm slate, readable -->
  <text x="236" y="151"
        font-family="{FONT}" font-size="15" font-weight="400"
        letter-spacing="2.0" fill="#7A8898">DUCKDB · EXTENSIONS</text>
</svg>"""
    _save(svg, "logo4-rust-split")


# ── Logo 5: Square App Icon — duck + water + wordmark ─────────────────────────

def logo5():
    W = H = 400
    # Duck precisely centred: body at (200, 162), s=1.02
    # Ripples centred at x=200 (matched to duck body, not offset)
    # Wordmark: split-x=228 centres "quack-rs" at ≈ x=200 at 50px bold
    # y=340: cap-top≈304, bottom≈357 → 43 px bottom padding (matches feel)

    DK_CX = 200   # duck & ripple centre-x

    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="55%" y2="100%">
      <stop offset="0%"   stop-color="#111927"/>
      <stop offset="100%" stop-color="#060C14"/>
    </linearGradient>
    <radialGradient id="dg" cx="62%" cy="24%" r="68%">
      <stop offset="0%"   stop-color="{GOLD_HI}"/>
      <stop offset="38%"  stop-color="{GOLD_MID}"/>
      <stop offset="100%" stop-color="{GOLD_SHA}"/>
    </radialGradient>
    <!-- Subtle radial glow behind duck -->
    <radialGradient id="glow" cx="50%" cy="50%" r="50%">
      <stop offset="0%"   stop-color="#F5C840" stop-opacity="0.07"/>
      <stop offset="100%" stop-color="#F5C840" stop-opacity="0"/>
    </radialGradient>
    <filter id="ds" x="-30%" y="-30%" width="190%" height="190%">
      <feDropShadow dx="0" dy="10" stdDeviation="16"
                    flood-color="#000" flood-opacity="0.62"/>
    </filter>
  </defs>

  <!-- Rounded-square background -->
  <rect width="{W}" height="{H}" rx="68" fill="url(#bg)"/>

  <!-- Warm glow centred behind duck -->
  <ellipse cx="{DK_CX}" cy="175" rx="140" ry="100" fill="url(#glow)"/>

  {duck(DK_CX, 162, s=1.02)}

  <!-- Water ripples — all centred on duck body cx -->
  <ellipse cx="{DK_CX}" cy="222" rx="88" ry="8.0"
           fill="none" stroke="#243C58" stroke-width="2.2" opacity="0.68"/>
  <ellipse cx="{DK_CX}" cy="233" rx="62" ry="5.8"
           fill="none" stroke="#243C58" stroke-width="1.6" opacity="0.46"/>
  <ellipse cx="{DK_CX}" cy="242" rx="38" ry="3.8"
           fill="none" stroke="#243C58" stroke-width="1.0" opacity="0.28"/>

  <!-- Wordmark centred: "quack" anchor=end / "-rs" anchor=start at x=228 -->
  <text x="228" y="340"
        font-family="{FONT}" font-size="50" font-weight="700"
        letter-spacing="-1.5" text-anchor="end"
        fill="{TEXT_LT}">quack</text>
  <text x="228" y="340"
        font-family="{FONT}" font-size="50" font-weight="700"
        letter-spacing="-1.5" text-anchor="start"
        fill="{RUST}">-rs</text>
</svg>"""
    _save(svg, "logo5-app-icon")


# ── Helpers ────────────────────────────────────────────────────────────────────

def _save(svg_src: str, name: str):
    svg_path = os.path.join(OUT, f"{name}.svg")
    png_path = os.path.join(OUT, f"{name}.png")
    with open(svg_path, "w") as f:
        f.write(svg_src)
    cairosvg.svg2png(url=svg_path, write_to=png_path, scale=2)
    print(f"  +  {name}.png  ({os.path.getsize(png_path) // 1024} KB)")


if __name__ == "__main__":
    print("Generating quack-rs logos …")
    logo1()
    logo2()
    logo3()
    logo4()
    logo5()
    print("Done.")
