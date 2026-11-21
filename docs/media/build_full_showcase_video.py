#!/usr/bin/env python3
"""
nodewipe extended full-feature showcase video generator (2 minutes, 1080p).
Combines:
  - Real terminal PTY recording (asciinema + agg)
  - Full-resolution Interactive TUI slides (tui-main.png, tui-dryrun.png)
  - Full-resolution Desktop GUI slides (gui-overview.png)
  - Embedded bottom caption cards on every scene
"""

import sys
import subprocess
from PIL import Image, ImageDraw, ImageFont, ImageSequence

WIDTH = 1920
HEIGHT = 1080
FPS = 24

FONT_MONO = ImageFont.truetype('/usr/share/fonts/TTF/MesloLGLNerdFontMono-Regular.ttf', 20)
FONT_TITLE = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 44)
FONT_SUBTITLE = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 24)
FONT_CAPTION = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 22)
FONT_BADGE = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 17)


def draw_caption_bar(draw, badge, text):
    # Caption container with dark translucent styling and purple accent border
    draw.rounded_rectangle([80, 955, 1840, 1045], radius=14, fill='#111422', outline='#8b5cf6', width=2)
    # Badge pill
    badge_w = int(draw.textlength(badge, font=FONT_BADGE)) + 36
    draw.rounded_rectangle([105, 972, 105 + badge_w, 1028], radius=8, fill='#8b5cf6')
    draw.text((105 + badge_w // 2, 1000), badge, font=FONT_BADGE, fill='#ffffff', anchor='mm')
    # Caption text
    draw.text((105 + badge_w + 24, 1000), text, font=FONT_CAPTION, fill='#f1f5f9', anchor='lm')


def create_slide(title, subtitle, bullets, badge, caption):
    img = Image.new('RGB', (WIDTH, HEIGHT), color='#0b0d13')
    draw = ImageDraw.Draw(img)

    # Ambient card border
    draw.rounded_rectangle([160, 100, 1760, 900], radius=24, fill='#12151f', outline='#312e81', width=3)

    # Title & Subtitle
    draw.text((960, 220), title, font=FONT_TITLE, fill='#a855f7', anchor='mm')
    draw.text((960, 290), subtitle, font=FONT_SUBTITLE, fill='#e2e8f0', anchor='mm')
    draw.line([280, 350, 1640, 350], fill='#374151', width=2)

    # Bullets
    y = 420
    for bullet in bullets:
        draw.ellipse([320, y - 8, 332, y + 4], fill='#38bdf8')
        draw.text((355, y), bullet, font=FONT_SUBTITLE, fill='#cbd5e1', anchor='lm')
        y += 75

    draw_caption_bar(draw, badge, caption)
    return img


def create_screenshot_slide(img_path, badge, caption):
    src = Image.open(img_path).convert('RGB')
    if src.size != (WIDTH, HEIGHT):
        src = src.resize((WIDTH, HEIGHT), Image.Resampling.LANCZOS)
    draw = ImageDraw.Draw(src)
    draw_caption_bar(draw, badge, caption)
    return src


def create_terminal_frame(term_frame, badge, caption):
    canvas = Image.new('RGB', (WIDTH, HEIGHT), color='#0a0c12')
    # Terminal frame width=888, height=630
    # Scale up terminal frame slightly for 1080p canvas: 1.25x -> 1110x787
    tw, th = term_frame.size
    scale = 1.3
    nw, nh = int(tw * scale), int(th * scale)
    scaled_term = term_frame.resize((nw, nh), Image.Resampling.BILINEAR)

    # Center horizontally, place in upper-middle
    x = (WIDTH - nw) // 2
    y = 65

    # Draw subtle window shadow/border
    draw = ImageDraw.Draw(canvas)
    draw.rounded_rectangle([x - 8, y - 8, x + nw + 8, y + nh + 8], radius=12, fill='#1b1f2e', outline='#2b3245', width=2)
    canvas.paste(scaled_term, (x, y))

    draw_caption_bar(draw, badge, caption)
    return canvas


def main():
    output_mp4 = sys.argv[1] if len(sys.argv) > 1 else 'docs/media/nodewipe-demo.mp4'

    cmd = [
        'ffmpeg', '-y',
        '-f', 'rawvideo',
        '-vcodec', 'rawvideo',
        '-s', f'{WIDTH}x{HEIGHT}',
        '-pix_fmt', 'rgb24',
        '-r', str(FPS),
        '-i', '-',
        '-c:v', 'libx264',
        '-pix_fmt', 'yuv420p',
        '-preset', 'fast',
        '-crf', '18',
        output_mp4
    ]

    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)

    def write_frame(img, count=1):
        raw = img.tobytes()
        for _ in range(count):
            proc.stdin.write(raw)

    print(">>> [1/7] Scene 1: Title Slide (10s)...")
    slide1 = create_slide(
        title="nodewipe v0.9.9",
        subtitle="The Surgical, Blazing-Fast Dev Environment Cleaner in Rust",
        bullets=[
            "Reclaims tens of gigabytes across 39+ ecosystems (Node, Rust, Python, Go, Java, AI)",
            "Parallel directory traversal powered by Rayon (multi-threaded work-stealing)",
            "Physical block accounting (st_blocks * 512) prevents logical byte illusions",
            "Marker-based validation guarantees sibling safety (no false positives)",
            "Zero-risk deletion: OS Trash fallback & instant Copy-on-Write (CoW) snapshots"
        ],
        badge="OVERVIEW",
        caption="A compiled Rust tool designed to solve multi-ecosystem workspace bloat safely."
    )
    write_frame(slide1, count=FPS * 10)

    print(">>> [2/7] Scene 2: Real Terminal Demo (Scan & Explain, 24s)...")
    # Load all 101 frames from the real terminal recording
    term_gif = Image.open('docs/media/nodewipe-demo.gif')
    term_frames = [f.copy().convert('RGB') for f in ImageSequence.Iterator(term_gif)]
    total_tf = len(term_frames)

    # Frame slices:
    # 0 - 35: scan command
    # 35 - 65: explain command
    # 65 - 80: tree command
    # 80 - 92: delete dry-run
    # 92 - 101: doctor command

    # Scan & Explain
    for i in range(0, min(35, total_tf)):
        f = create_terminal_frame(term_frames[i],
                                  badge="1/6 WORKSPACE SCAN",
                                  caption="Rayon parallel traversal scans multi-language workspace trees in seconds.")
        write_frame(f, count=8)

    for i in range(35, min(65, total_tf)):
        f = create_terminal_frame(term_frames[i],
                                  badge="2/6 DECISION INTELLIGENCE",
                                  caption="nodewipe explain evaluates git branch staleness, markers, and rebuild commands.")
        write_frame(f, count=8)

    print(">>> [3/7] Scene 3: Real Terminal Demo (Tree & Dry-Run Delete, 18s)...")
    for i in range(65, min(80, total_tf)):
        f = create_terminal_frame(term_frames[i],
                                  badge="3/6 STORAGE TREE",
                                  caption="Storage Explorer inspects internal subpackage weights and debug symbol bloat.")
        write_frame(f, count=8)

    for i in range(80, min(92, total_tf)):
        f = create_terminal_frame(term_frames[i],
                                  badge="4/6 SAFE EXCISION",
                                  caption="Safe by default: OS Trash recovery and instant zero-space CoW reflink snapshot backups.")
        write_frame(f, count=8)

    print(">>> [4/7] Scene 4: Real Terminal Demo (System Doctor, 16s)...")
    for i in range(92, total_tf):
        f = create_terminal_frame(term_frames[i],
                                  badge="5/6 SYSTEM DOCTOR",
                                  caption="System Doctor audits toolchain bloat (Rust, Node, Python, AI models) and cache health.")
        write_frame(f, count=16)

    print(">>> [5/7] Scene 5: Interactive TUI Showcase (20s)...")
    tui_main = create_screenshot_slide(
        'docs/screenshots/tui-main.png',
        badge="6/7 INTERACTIVE TUI",
        caption="Ratatui-powered terminal interface with keyboard navigation, sort filters, and live stats."
    )
    write_frame(tui_main, count=FPS * 10)

    tui_dryrun = create_screenshot_slide(
        'docs/screenshots/tui-dryrun.png',
        badge="6/7 TUI DRY-RUN MODE",
        caption="Press 'P' to simulate deletions and inspect candidate selections before touching a single byte."
    )
    write_frame(tui_dryrun, count=FPS * 10)

    print(">>> [6/7] Scene 6: Desktop GUI Showcase (18s)...")
    gui_slide = create_screenshot_slide(
        'docs/screenshots/gui-overview.png',
        badge="7/7 DESKTOP GUI",
        caption="Native, responsive Tauri v2 desktop application sharing the exact same compiled Rust engine."
    )
    write_frame(gui_slide, count=FPS * 18)

    print(">>> [7/7] Scene 7: Outro & Installation (14s)...")
    outro = create_slide(
        title="Get Started with nodewipe",
        subtitle="Reclaim Your Disk. Keep Your Workflow.",
        bullets=[
            "Arch Linux (AUR):       yay -S nodewipe",
            "Homebrew (macOS/Linux): brew install KADHIRAVANEG/nodewipe/nodewipe",
            "NPM (Zero-Rust shim):   npx @joker53/nodewipe",
            "GitHub Repository:      https://github.com/KADHIRAVANEG/nodewipe",
            "Free & Open Source under MIT. Star on GitHub to support the project!"
        ],
        badge="INSTALL",
        caption="Cross-platform native binaries with verified SHA-256 sums are available now."
    )
    write_frame(outro, count=FPS * 14)

    proc.stdin.close()
    proc.wait()
    print(f"\n[SUCCESS] Extended full-feature showcase video generated: {output_mp4}")


if __name__ == '__main__':
    main()
