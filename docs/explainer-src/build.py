#!/usr/bin/env python3
"""Builds the JAMX explainer page. Diagrams are generated here so that the
coordinates stay on a grid."""
import sys

def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

MARKERS = '''<defs>
<marker id="ar" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" class="mk"/></marker>
<marker id="arm" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" class="mkm"/></marker>
</defs>'''

def text(x, y, s, cls="t", anchor="start"):
    return f'<text x="{x}" y="{y}" class="{cls}" text-anchor="{anchor}">{esc(s)}</text>'

# ---------------------------------------------------------------- figure 1: blast radius
def fig1():
    o = [f'<svg class="dg" viewBox="0 0 880 300" role="img" aria-label="With custody, one stolen key reaches all twelve customers of the vault; in JAMX it reaches one.">']
    o.append(text(220, 28, "Custodial exchange", "tb", "middle"))
    o.append('<rect x="60" y="46" width="320" height="176" rx="10" class="bx-d"/>')
    o.append(text(220, 240, "one vault, one key", "tm", "middle"))
    for r in range(3):
        for c in range(4):
            o.append(f'<rect x="{84 + 72 * c}" y="{70 + 48 * r}" width="56" height="30" rx="5" class="cell-d"/>')
    o.append(text(220, 280, "stolen key: 12 of 12 lose", "td", "middle"))
    o.append('<line x1="440" y1="20" x2="440" y2="285" class="ln-m"/>')
    o.append(text(660, 28, "JAMX", "tb", "middle"))
    for r in range(3):
        for c in range(4):
            x, y = 524 + 72 * c, 70 + 48 * r
            hit = (r, c) == (1, 2)
            o.append(f'<rect x="{x}" y="{y}" width="56" height="30" rx="5" class="{"cell-d" if hit else "bx"}"/>')
            kc = "kd" if hit else "k"
            o.append(f'<circle cx="{x + 44}" cy="{y + 15}" r="4" class="{kc}"/><line x1="{x + 26}" y1="{y + 15}" x2="{x + 40}" y2="{y + 15}" class="{kc}"/><line x1="{x + 30}" y1="{y + 15}" x2="{x + 30}" y2="{y + 20}" class="{kc}"/>')
    o.append(text(660, 240, "each balance under its owner's key", "tm", "middle"))
    o.append(text(660, 280, "stolen key: 1 of 12 loses", "tb", "middle"))
    o.append('</svg>')
    return "\n".join(o)

# ---------------------------------------------------------------- figure 2: path of a trade
def fig2():
    boxes = [
        (20, "bx", "1 · You sign", ["an order with a limit", "price, on your own", "device"]),
        (208, "bx", "2 · Someone bundles", ["the round's orders into", "a batch; it touches", "no balance"]),
        (396, "bx-a", "3 · Refine", ["checks signatures,", "sorts by price, drops", "what cannot cross"]),
        (584, "bx-a", "4 · Accumulate", ["sees balances, finds", "one price, swaps", "balance for balance"]),
        (772, "bx", "5 · Done", ["your balance changed,", "under your own key"]),
    ]
    o = ['<svg class="dg" viewBox="0 0 940 262" role="img" aria-label="The path of a trade: signed order, batch, refine, accumulate, updated balance.">', MARKERS]
    for x, cls, title, lines in boxes:
        o.append(f'<rect x="{x}" y="70" width="148" height="112" rx="8" class="{cls}"/>')
        o.append(text(x + 74, 96, title, "tb", "middle"))
        for i, l in enumerate(lines):
            o.append(text(x + 74, 120 + 17 * i, l, "tm", "middle"))
    labels = ["signed order", "batch", "crossing book, up to 48 KiB", "new balances"]
    for i, lab in enumerate(labels):
        x1 = 20 + 188 * i + 148
        o.append(f'<line x1="{x1 + 3}" y1="126" x2="{x1 + 37}" y2="126" class="ln" marker-end="url(#ar)"/>')
        o.append(text(x1 + 20, 56, lab, "tm", "middle"))
    def bracket(x1, x2, lines):
        s = f'<path d="M{x1},196 v8 H{x2} v-8" class="ln-b"/>'
        for i, l in enumerate(lines):
            s += text((x1 + x2) / 2, 224 + 16 * i, l, "tm", "middle")
        return s
    o.append(bracket(20, 356, ["off-chain: nothing has changed yet"]))
    o.append(bracket(396, 544, ["in parallel; re-run by", "randomly chosen validators"]))
    o.append(bracket(584, 920, ["by every validator: all or nothing"]))
    o.append('</svg>')
    return "\n".join(o)

# ---------------------------------------------------------------- figure 3: buying bitcoin
def fig3():
    o = ['<svg class="dg" viewBox="0 0 940 372" role="img" aria-label="Buying bitcoin: the reais wait locked on JAM, the bitcoin is paid on the Bitcoin network, someone carries the proof to JAM and the reais are released. No arrow goes from JAM to Bitcoin.">', MARKERS]
    o.append('<rect x="20" y="36" width="900" height="104" rx="10" class="band-w"/>')
    o.append('<rect x="20" y="234" width="900" height="104" rx="10" class="band-a"/>')
    o.append(text(36, 60, "Bitcoin network", "tb"))
    o.append(text(36, 258, "JAMX, on JAM", "tb"))
    # step 2 on bitcoin
    o.append('<rect x="290" y="56" width="230" height="64" rx="8" class="bx"/>')
    o.append(text(405, 82, "2 · Alice pays 0.001 BTC", "tb", "middle"))
    o.append(text(405, 102, "from her wallet to Bob's address", "tm", "middle"))
    # step 1, 4, 5 on JAM
    o.append('<rect x="150" y="254" width="200" height="64" rx="8" class="bx"/>')
    o.append(text(250, 280, "1 · Bob locks R$ 600", "tb", "middle"))
    o.append(text(250, 300, "Alice locks a 10% bond", "tm", "middle"))
    o.append('<rect x="470" y="254" width="220" height="64" rx="8" class="bx"/>')
    o.append(text(580, 280, "4 · The program redoes the sums", "tb", "middle"))
    o.append(text(580, 300, "work, difficulty, blocks on top", "tm", "middle"))
    o.append('<rect x="730" y="254" width="176" height="64" rx="8" class="bx-a"/>')
    o.append(text(818, 280, "5 · Releases", "tb", "middle"))
    o.append(text(818, 300, "R$ 600 + bond → Alice", "tm", "middle"))
    # arrows
    o.append('<line x1="353" y1="286" x2="466" y2="286" class="ln-m" marker-end="url(#arm)"/>')
    o.append(text(410, 276, "waiting for proof", "tm", "middle"))
    o.append('<polyline points="523,88 600,88 600,250" class="ln" marker-end="url(#ar)"/>')
    o.append(text(612, 178, "3 · anyone carries", "t"))
    o.append(text(612, 196, "block headers + a receipt", "tm"))
    o.append('<line x1="693" y1="286" x2="726" y2="286" class="ln" marker-end="url(#ar)"/>')
    # the arrow that does not exist
    o.append('<line x1="110" y1="228" x2="110" y2="150" class="ln-m" marker-end="url(#arm)"/>')
    o.append('<path d="M100,178 l20,20 M120,178 l-20,20" class="ln-d"/>')
    o.append(text(128, 184, "JAM cannot write", "td"))
    o.append(text(128, 200, "to Bitcoin", "td"))
    o.append(text(150, 360, "If Bitcoin moves on and no proof arrives, the R$ 600 go back to Bob, together with Alice's bond.", "tm"))
    o.append('</svg>')
    return "\n".join(o)

# ---------------------------------------------------------------- figure 4: the bottleneck
def fig4():
    rows = [
        ("Refine can authenticate", "1,650 orders per package", "5 billion gas, 3.03 million per order", 1650, "bar"),
        ("The 48 KiB output holds", "about 770 orders", "60 bytes per order", 770, "bar"),
        ("Accumulate can settle", "about 290 orders", "10 million gas, 29 thousand per order, a safety reserve", 290, "bx-a"),
    ]
    o = ['<svg class="dg" viewBox="0 0 880 236" role="img" aria-label="Capacity per work-package: refine 1,650 orders, output 770, accumulate 290. Accumulate is the bottleneck.">']
    for i, (a, b, c, n, cls) in enumerate(rows):
        y = 34 + 70 * i
        w = round(n * 820 / 1650)
        o.append(text(20, y, a, "tb"))
        o.append(f'<rect x="20" y="{y + 10}" width="{w}" height="26" rx="5" class="{cls}"/>')
        tx = 20 + w + 12 if w < 420 else 20 + w - 12
        anchor = "start" if w < 420 else "end"
        o.append(text(tx, y + 28, f"{b}  ({c})", "tm" if w < 420 else "t", anchor))
    o.append('</svg>')
    return "\n".join(o)

# ---------------------------------------------------------------- figure 5: who redoes the work
def fig5():
    panels = [
        (20, "Ethereum, base layer", "all", 7, ["everyone re-executes everything", "low ceiling per block", "high fee per trade"], ("no extra key", "ok")),
        (255, "Ethereum rollup", "one", 7, ["one sequencer executes", "validity proof or fraud window", "cheap and fast"], ("upgrade council + bridge", "bad")),
        (490, "Solana", "all", 9, ["everyone re-executes everything", "large machines, 0.4 s blocks", "ceiling per transaction"], ("no extra key", "ok")),
        (725, "JAM", "jam", 7, ["a few execute the heavy work,", "randomly chosen ones re-run it,", "everyone applies the result"], ("no extra key", "ok")),
    ]
    jam_fill, jam_ring = {0, 1, 2}, {6, 9, 12, 14, 17, 19}
    o = ['<svg class="dg" viewBox="0 0 960 322" role="img" aria-label="Who executes the heavy computation: on Ethereum and Solana, every validator; on a rollup, one sequencer; on JAM, a few guarantors and randomly chosen auditors. Only the rollup puts an extra key between the user and the funds.">']
    for x0, title, mode, r, lines, (chip, kind) in panels:
        o.append(text(x0 + 107, 26, title, "tb", "middle"))
        for i in range(20):
            row, col = divmod(i, 5)
            cx, cy = x0 + 47 + 30 * col, 56 + 30 * row
            if mode == "all" or (mode == "one" and i == 0) or (mode == "jam" and i in jam_fill):
                cls = "dot"
            elif mode == "jam" and i in jam_ring:
                cls = "dot-r"
            else:
                cls = "dot-o"
            o.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" class="{cls}"/>')
        for i, l in enumerate(lines):
            o.append(text(x0 + 107, 190 + 17 * i, l, "tm", "middle"))
        o.append(f'<rect x="{x0 + 12}" y="238" width="190" height="26" rx="13" class="{"chip-bad" if kind == "bad" else "chip-ok"}"/>')
        o.append(text(x0 + 107, 255, chip, "td" if kind == "bad" else "tm", "middle"))
    lx = 20
    o.append(f'<circle cx="{lx + 7}" cy="298" r="6" class="dot"/>' + text(lx + 20, 302, "executes the heavy computation", "tm"))
    o.append(f'<circle cx="{lx + 247}" cy="298" r="6" class="dot-r"/>' + text(lx + 260, 302, "re-runs it, chosen by lottery", "tm"))
    o.append(f'<circle cx="{lx + 467}" cy="298" r="6" class="dot-o"/>' + text(lx + 480, 302, "only checks or applies the result", "tm"))
    o.append('</svg>')
    return "\n".join(o)

page = open(sys.argv[1], encoding="utf-8").read()
for name, fn in (("FIG1", fig1), ("FIG2", fig2), ("FIG3", fig3), ("FIG4", fig4), ("FIG5", fig5)):
    assert "{{" + name + "}}" in page, name
    page = page.replace("{{" + name + "}}", fn())
open(sys.argv[2], "w", encoding="utf-8").write(page)
print("wrote", sys.argv[2], len(page), "bytes")
