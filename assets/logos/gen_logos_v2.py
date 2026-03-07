"""
Generate professional quack-rs logos via hand-crafted SVG → cairosvg → PNG.

Key principles:
 - NO stroke outlines on duck body parts  (overlapping shapes merge invisibly)
 - Radial gradients for depth/lighting
 - tspan for wordmarks (avoids font-metric-dependent text gaps)
 - Accurate drop-shadows and composition
"""
import cairosvg, os

OUT = os.path.dirname(os.path.abspath(__file__))

# ─── Duck builder ────────────────────────────────────────────────────────────

def duck(cx, cy, s=1.0,
         body_color="#F2B822", dark_color="#C88E08",
         bill_color="#E06010", eye_color="#0D1117",
         grad_id="dg", shadow_filter="url(#ds)"):
    """
    Return SVG markup for a swimming duck facing right.
    cx,cy = body centre.  s = scale (1.0 → ~140×100px bounding box).
    All body parts: stroke:none, same gradient fill → seamless silhouette.
    """
    bx, by = cx, cy
    rx, ry = 68*s, 46*s
    hx, hy = cx+52*s, cy-42*s
    hr      = 32*s

    # Tail feather: curved up-left from body's left edge
    tx0 = cx - rx
    tail = (
        f"M {tx0:.1f},{by:.1f} "
        f"C {cx-82*s:.1f},{by-18*s:.1f} {cx-88*s:.1f},{by-44*s:.1f} {cx-74*s:.1f},{by-56*s:.1f} "
        f"C {cx-60*s:.1f},{by-60*s:.1f} {cx-52*s:.1f},{by-44*s:.1f} {cx-56*s:.1f},{by-22*s:.1f} "
        f"C {cx-60*s:.1f},{by-8*s:.1f}  {cx-66*s:.1f},{by-2*s:.1f}  {tx0:.1f},{by:.1f} Z"
    )

    # Bill: smooth rounded leaf shape pointing right
    b1x, b1y = hx+24*s, hy-9*s
    b2x, b2y = hx+58*s, hy+1*s
    b3x, b3y = hx+24*s, hy+13*s
    bill = (f"M {b1x:.1f},{b1y:.1f} "
            f"Q {b2x+5*s:.1f},{b1y:.1f} {b2x:.1f},{b2y:.1f} "
            f"Q {b2x+5*s:.1f},{b3y:.1f} {b3x:.1f},{b3y:.1f} "
            f"Q {hx+18*s:.1f},{hy:.1f} {b1x:.1f},{b1y:.1f} Z")

    # Eye
    ex, ey, er = hx+5*s, hy-10*s, 6.5*s

    # Wing arc
    wx0, wy0 = cx-18*s, cy+8*s
    wx1, wy1 = cx+18*s, cy-13*s
    wx2, wy2 = cx+50*s, cy+8*s

    fill_ref = f"url(#{grad_id})"

    return f"""
  <g filter="{shadow_filter}">
    <path d="{tail}" fill="{fill_ref}" stroke="none"/>
    <ellipse cx="{bx:.1f}" cy="{by:.1f}" rx="{rx:.1f}" ry="{ry:.1f}"
             transform="rotate(-4,{bx:.1f},{by:.1f})"
             fill="{fill_ref}" stroke="none"/>
    <circle  cx="{hx:.1f}" cy="{hy:.1f}" r="{hr:.1f}"
             fill="{fill_ref}" stroke="none"/>
    <path d="{bill}" fill="{bill_color}" stroke="none"/>
    <circle  cx="{ex:.1f}" cy="{ey:.1f}" r="{er:.1f}" fill="{eye_color}" stroke="none"/>
    <circle  cx="{ex+2.5*s:.1f}" cy="{ey-2.8*s:.1f}" r="{2.5*s:.1f}" fill="white" stroke="none"/>
    <path d="M {wx0:.1f},{wy0:.1f} Q {wx1:.1f},{wy1:.1f} {wx2:.1f},{wy2:.1f}"
          fill="none" stroke="{dark_color}" stroke-width="{2.8*s:.1f}"
          stroke-linecap="round" opacity="0.55"/>
  </g>"""


# ─── Logo 1: Dark Elegant — primary README horizontal lockup ─────────────────
def logo1():
    W, H = 660, 210
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="dg" cx="58%" cy="32%" r="65%">
      <stop offset="0%"   stop-color="#FFE87A"/>
      <stop offset="100%" stop-color="#C08A06"/>
    </radialGradient>
    <filter id="ds" x="-25%" y="-25%" width="170%" height="170%">
      <feDropShadow dx="0" dy="6" stdDeviation="12"
                    flood-color="#000000" flood-opacity="0.50"/>
    </filter>
    <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%"   stop-color="#0D1117"/>
      <stop offset="100%" stop-color="#161B22"/>
    </linearGradient>
  </defs>

  <rect width="{W}" height="{H}" rx="22" fill="url(#bg)"/>

  <!-- Duck -->
  <g transform="translate(20,6)">{duck(100, 118, s=0.92)}</g>

  <!-- Wordmark: quack-rs in one <text> with tspan → no gap -->
  <text x="224" y="126"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="80" font-weight="800" letter-spacing="-2">
    <tspan fill="#F0F6FC">quack</tspan><tspan fill="#CE422B">-rs</tspan>
  </text>

  <!-- Tagline -->
  <text x="226" y="157"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="17" letter-spacing="2.5" fill="#4B6080">DUCKDB  ·  RUST  ·  FAST</text>
</svg>"""
    _save(svg, "logo1-dark-elegant")


# ─── Logo 2: Light / Docs ─────────────────────────────────────────────────────
def logo2():
    W, H = 660, 210
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="dg" cx="55%" cy="30%" r="65%">
      <stop offset="0%"   stop-color="#FFD54F"/>
      <stop offset="100%" stop-color="#B8780A"/>
    </radialGradient>
    <filter id="ds" x="-25%" y="-25%" width="170%" height="170%">
      <feDropShadow dx="0" dy="4" stdDeviation="8"
                    flood-color="#9C7700" flood-opacity="0.22"/>
    </filter>
  </defs>

  <rect width="{W}" height="{H}" rx="18" fill="#FFFFFF"/>
  <rect width="{W}" height="{H}" rx="18" fill="none"
        stroke="#E2E8F0" stroke-width="1.5"/>

  <!-- Duck -->
  <g transform="translate(10,8)">{duck(96, 118, s=0.90, eye_color="#1A1A1A")}</g>

  <!-- Wordmark -->
  <text x="212" y="126"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="80" font-weight="800" letter-spacing="-2">
    <tspan fill="#0D1117">quack</tspan><tspan fill="#CE422B">-rs</tspan>
  </text>

  <!-- Tagline -->
  <text x="214" y="158"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="17" letter-spacing="0.5" fill="#6B7280">
    DuckDB extensions in Rust
  </text>
</svg>"""
    _save(svg, "logo2-light-docs")


# ─── Logo 3: Circular Badge Icon ─────────────────────────────────────────────
def logo3():
    W = H = 360
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="bgr" cx="40%" cy="35%" r="65%">
      <stop offset="0%"   stop-color="#1C2330"/>
      <stop offset="100%" stop-color="#080C14"/>
    </radialGradient>
    <radialGradient id="dg" cx="60%" cy="28%" r="65%">
      <stop offset="0%"   stop-color="#FFE870"/>
      <stop offset="100%" stop-color="#B88008"/>
    </radialGradient>
    <linearGradient id="ring" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%"   stop-color="#F5C540"/>
      <stop offset="100%" stop-color="#CE422B"/>
    </linearGradient>
    <filter id="ds" x="-30%" y="-30%" width="180%" height="180%">
      <feDropShadow dx="0" dy="6" stdDeviation="12"
                    flood-color="#000" flood-opacity="0.55"/>
    </filter>
    <clipPath id="circ">
      <circle cx="{W//2}" cy="{H//2}" r="{W//2-2}"/>
    </clipPath>
  </defs>

  <circle cx="{W//2}" cy="{H//2}" r="{W//2}" fill="url(#bgr)"/>
  <circle cx="{W//2}" cy="{H//2}" r="{W//2-9}"
          fill="none" stroke="url(#ring)" stroke-width="5"/>

  <!-- Duck: centred slightly above middle to leave room for label -->
  <g clip-path="url(#circ)">
    {duck(172, 160, s=0.86)}
  </g>

  <!-- Label at bottom -->
  <!-- Split at 199 so full "quack-rs" is visually centred in the 360px badge -->
  <text x="199" y="{H-42}"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="34" font-weight="800" letter-spacing="-0.5"
        text-anchor="end" fill="#F0F6FC">quack</text>
  <text x="199" y="{H-42}"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="34" font-weight="800" letter-spacing="-0.5"
        text-anchor="start" fill="#CE422B">-rs</text>
</svg>"""
    _save(svg, "logo3-badge-icon")


# ─── Logo 4: Rust Split — colour-block horizontal ────────────────────────────
def logo4():
    W, H = 660, 210
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <radialGradient id="dg" cx="55%" cy="28%" r="65%">
      <stop offset="0%"   stop-color="#FFF0A0"/>
      <stop offset="100%" stop-color="#E0A010"/>
    </radialGradient>
    <filter id="ds" x="-25%" y="-25%" width="170%" height="170%">
      <feDropShadow dx="0" dy="5" stdDeviation="10"
                    flood-color="#5A0800" flood-opacity="0.45"/>
    </filter>
    <clipPath id="full">
      <rect width="{W}" height="{H}" rx="22"/>
    </clipPath>
  </defs>

  <g clip-path="url(#full)">
    <rect x="0"   y="0" width="205" height="{H}" fill="#B02810"/>
    <rect x="205" y="0" width="{W-205}" height="{H}" fill="#18100E"/>
  </g>
  <!-- Clean edge between the two blocks -->
  <line x1="205" y1="0" x2="205" y2="{H}" stroke="#7A1808" stroke-width="1.5"/>

  <!-- Duck spanning both colour blocks -->
  <g transform="translate(10,5)">{duck(105, 118, s=0.92,
      body_color="#F5D050", dark_color="#C8A020", bill_color="#FF9020")}</g>

  <!-- Wordmark on dark panel -->
  <text x="228" y="126"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="80" font-weight="800" letter-spacing="-2">
    <tspan fill="#F0E0D8">quack</tspan><tspan fill="#FF6644">-rs</tspan>
  </text>

  <text x="230" y="158"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="16" letter-spacing="2" fill="#5A4040">DUCKDB  ·  EXTENSIONS</text>
</svg>"""
    _save(svg, "logo4-rust-split")


# ─── Logo 5: Square App Icon — duck with water ripples ───────────────────────
def logo5():
    W = H = 400
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
  <defs>
    <linearGradient id="bg" x1="0%" y1="0%" x2="60%" y2="100%">
      <stop offset="0%"   stop-color="#111927"/>
      <stop offset="100%" stop-color="#060C14"/>
    </linearGradient>
    <radialGradient id="dg" cx="60%" cy="28%" r="65%">
      <stop offset="0%"   stop-color="#FFE870"/>
      <stop offset="100%" stop-color="#C08A06"/>
    </radialGradient>
    <radialGradient id="glow" cx="50%" cy="55%" r="48%">
      <stop offset="0%"   stop-color="#F5C842" stop-opacity="0.08"/>
      <stop offset="100%" stop-color="#F5C842" stop-opacity="0"/>
    </radialGradient>
    <filter id="ds" x="-30%" y="-30%" width="190%" height="190%">
      <feDropShadow dx="0" dy="10" stdDeviation="16"
                    flood-color="#000" flood-opacity="0.60"/>
    </filter>
  </defs>

  <rect width="{W}" height="{H}" rx="68" fill="url(#bg)"/>

  <!-- Duck centred -->
  {duck(W//2+4, H//2-30, s=1.02)}

  <!-- Water ripples snug under the duck body -->
  <ellipse cx="{W//2-4}" cy="238" rx="90" ry="9"
           fill="none" stroke="#243C58" stroke-width="2.2" opacity="0.65"/>
  <ellipse cx="{W//2-4}" cy="248" rx="64" ry="6.5"
           fill="none" stroke="#243C58" stroke-width="1.6" opacity="0.45"/>
  <ellipse cx="{W//2-4}" cy="256" rx="40" ry="4.5"
           fill="none" stroke="#243C58" stroke-width="1.0" opacity="0.30"/>

  <!-- Split anchored at 228 so the full "quack-rs" is visually centred -->
  <text x="228" y="{H-52}"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="50" font-weight="800" letter-spacing="-1.5"
        text-anchor="end" fill="#F0F6FC">quack</text>
  <text x="228" y="{H-52}"
        font-family="'Segoe UI','Helvetica Neue',Arial,sans-serif"
        font-size="50" font-weight="800" letter-spacing="-1.5"
        text-anchor="start" fill="#CE422B">-rs</text>
</svg>"""
    _save(svg, "logo5-app-icon")


# ─── Helpers ─────────────────────────────────────────────────────────────────

def _save(svg_src: str, name: str):
    svg_path = os.path.join(OUT, f"{name}.svg")
    png_path = os.path.join(OUT, f"{name}.png")
    with open(svg_path, "w") as f:
        f.write(svg_src)
    cairosvg.svg2png(url=svg_path, write_to=png_path, scale=2)
    print(f"  ✓  {name}.png  ({os.path.getsize(png_path)//1024} KB)")


if __name__ == "__main__":
    print("Generating logos …")
    logo1(); logo2(); logo3(); logo4(); logo5()
    print("Done.")
