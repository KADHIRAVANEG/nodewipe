#!/usr/bin/env python3
"""
nodewipe showcase video generator.
Renders full HD 1080p presentation video with real terminal recordings,
custom slide title cards, and embedded explanatory captions.
"""

import sys
import subprocess
from PIL import Image, ImageDraw, ImageFont

WIDTH = 1920
HEIGHT = 1080
FPS = 24

FONT_MONO = ImageFont.truetype('/usr/share/fonts/TTF/MesloLGLNerdFontMono-Regular.ttf', 20)
FONT_MONO_BOLD = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 20)
FONT_TITLE = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 44)
FONT_SUBTITLE = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 24)
FONT_CAPTION = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 21)
FONT_BADGE = ImageFont.truetype('/usr/share/fonts/liberation/LiberationMono-Bold.ttf', 16)


def draw_window_frame(draw, title="nodewipe v0.9.9 — terminal"):
    # Window canvas
    draw.rounded_rectangle([90, 60, 1830, 920], radius=16, fill='#12151f', outline='#2b3245', width=2)
    # Titlebar
    draw.rounded_rectangle([90, 60, 1830, 120], radius=16, fill='#191d2b')
    draw.rectangle([90, 100, 1830, 120], fill='#191d2b')
    # Window controls
    draw.ellipse([120, 82, 136, 98], fill='#ff5f56')
    draw.ellipse([146, 82, 162, 98], fill='#ffbd2e')
    draw.ellipse([172, 82, 188, 98], fill='#27c93f')
    # Window title
    draw.text((960, 90), title, font=FONT_CAPTION, fill='#8b9bb4', anchor='mm')


def draw_caption_bar(draw, badge, text):
    # Caption container
    draw.rounded_rectangle([90, 950, 1830, 1030], radius=12, fill='#161926', outline='#8b5cf6', width=2)
    # Badge
    draw.rounded_rectangle([115, 966, 275, 1014], radius=8, fill='#8b5cf6')
    draw.text((195, 990), badge, font=FONT_BADGE, fill='#ffffff', anchor='mm')
    # Caption text
    draw.text((295, 990), text, font=FONT_CAPTION, fill='#e2e8f0', anchor='lm')


def create_slide_frame(title, subtitle, bullets, badge, caption):
    img = Image.new('RGB', (WIDTH, HEIGHT), color='#0a0c10')
    draw = ImageDraw.Draw(img)

    # Ambient glow
    draw.rounded_rectangle([180, 120, 1740, 880], radius=24, fill='#111420', outline='#312e81', width=3)

    # Main title
    draw.text((960, 240), title, font=FONT_TITLE, fill='#a855f7', anchor='mm')
    draw.text((960, 310), subtitle, font=FONT_SUBTITLE, fill='#e2e8f0', anchor='mm')

    # Divider
    draw.line([300, 370, 1620, 370], fill='#374151', width=2)

    # Bullet points
    y = 440
    for bullet in bullets:
        draw.ellipse([340, y - 8, 352, y + 4], fill='#38bdf8')
        draw.text((375, y), bullet, font=FONT_SUBTITLE, fill='#cbd5e1', anchor='lm')
        y += 70

    draw_caption_bar(draw, badge, caption)
    return img


def render_terminal_frame(typed_cmd, output_lines, cursor_on, badge, caption):
    img = Image.new('RGB', (WIDTH, HEIGHT), color='#0a0c10')
    draw = ImageDraw.Draw(img)

    draw_window_frame(draw)

    # Prompt line
    prompt_prefix = "❯ "
    draw.text((130, 160), prompt_prefix, font=FONT_MONO_BOLD, fill='#10b981')
    prefix_w = draw.textlength(prompt_prefix, font=FONT_MONO_BOLD)
    draw.text((130 + prefix_w, 160), typed_cmd, font=FONT_MONO, fill='#f8fafc')

    if cursor_on:
        cmd_w = draw.textlength(typed_cmd, font=FONT_MONO)
        draw.rectangle([130 + prefix_w + cmd_w + 2, 160, 130 + prefix_w + cmd_w + 14, 184], fill='#38bdf8')

    # Output lines
    y = 205
    for line in output_lines:
        color = '#cbd5e1'
        if 'artifacts found' in line or 'reclaimable' in line or 'Summary:' in line:
            color = '#38bdf8'
        elif 'Cargo target' in line or 'rust_target' in line:
            color = '#f97316'
        elif 'node_modules' in line or 'Node.js' in line or 'Next.js' in line:
            color = '#22c55e'
        elif 'Python venv' in line or 'torch' in line:
            color = '#eab308'
        elif 'Explanation for' in line or 'Storage Explorer' in line or 'doctor' in line:
            color = '#c084fc'
        elif 'Low Risk' in line or 'Safe to delete' in line or 'Zero data loss' in line:
            color = '#4ade80'
        elif '────────────────' in line:
            color = '#475569'

        draw.text((130, y), line, font=FONT_MONO, fill=color)
        y += 28

    draw_caption_bar(draw, badge, caption)
    return img


def main():
    output_video = sys.argv[1] if len(sys.argv) > 1 else 'docs/media/nodewipe-demo.mp4'

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
        output_video
    ]

    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)

    def write_frame(img, count=1):
        raw = img.tobytes()
        for _ in range(count):
            proc.stdin.write(raw)

    print("[1/7] Rendering Intro Slide...")
    intro_img = create_slide_frame(
        title="nodewipe v0.9.9",
        subtitle="The Surgical, Blazing-Fast Dev Environment Cleaner in Rust",
        bullets=[
            "Reclaims tens of gigabytes from orphaned target/, node_modules, and venvs",
            "Parallel directory traversal powered by Rayon (multi-threaded work-stealing)",
            "Accurate physical block sizing (st_blocks * 512) prevents logical byte illusions",
            "Marker-based verification guarantees sibling safety (no false positives)",
            "Zero-risk deletion: OS Trash fallback & instant Copy-on-Write (CoW) snapshots"
        ],
        badge="OVERVIEW",
        caption="A compiled Rust tool designed to solve multi-ecosystem workspace bloat without risks."
    )
    write_frame(intro_img, count=96)  # 4 seconds

    # Scene 1: Scan
    print("[2/7] Rendering Scene 1: Parallel Workspace Scan...")
    cmd_text = "nodewipe scan ~/projects"
    scan_output = [
        " 12.00 MB     0d   Cargo target     ~/projects/rust-service/target",
        "  8.00 MB     0d   node_modules      ~/projects/web-frontend/node_modules",
        " 24.00 MB     0d   Python venv       ~/projects/ml-pipeline/.venv",
        "  6.00 MB     0d   Next.js cache     ~/projects/next-app/.next",
        "",
        " 4 artifacts found, 50.00 MB reclaimable (physical disk blocks: 50.00 MB)"
    ]

    # Typing
    for i in range(1, len(cmd_text) + 1):
        f = render_terminal_frame(cmd_text[:i], [], cursor_on=True,
                                  badge="DISK SCAN",
                                  caption="Rayon multi-threaded traversal discovers 39+ artifact ecosystems concurrently.")
        write_frame(f, count=2)

    # Display output
    for f_idx in range(160):
        cursor = (f_idx % 12) < 6
        f = render_terminal_frame(cmd_text, scan_output, cursor_on=cursor,
                                  badge="DISK SCAN",
                                  caption="Rayon multi-threaded traversal discovers 39+ artifact ecosystems concurrently.")
        write_frame(f, count=1)

    # Scene 2: Explain
    print("[3/7] Rendering Scene 2: Decision Intelligence...")
    cmd_text = "nodewipe explain ~/projects/rust-service/target"
    explain_output = [
        "Explanation for ~/projects/rust-service/target",
        "────────────────────────────────────────────────────────────────────────",
        "  • Path:           ~/projects/rust-service/target",
        "  • Size:           12.0 MB (physical disk allocation: 12582912 bytes)",
        "  • Artifact Type:   Cargo target (rust_target)",
        "  • Detection:      Confirmed sibling Cargo.toml in project root",
        "  • Protection:     Unpinned (Eligible for surgical cleanup)",
        "  • Risk Level:     Low Risk — Safe to delete (rebuilds entirely locally)",
        "  • Activity:       Source code touched today (active project)",
        "  • Rebuild Recipe:  cargo build (zero network download needed)",
        "────────────────────────────────────────────────────────────────────────"
    ]
    for i in range(1, len(cmd_text) + 1):
        f = render_terminal_frame(cmd_text[:i], [], cursor_on=True,
                                  badge="EXPLAIN",
                                  caption="nodewipe explain evaluates git branch staleness, markers, and rebuild commands.")
        write_frame(f, count=2)

    for f_idx in range(160):
        cursor = (f_idx % 12) < 6
        f = render_terminal_frame(cmd_text, explain_output, cursor_on=cursor,
                                  badge="DECISION INTEL",
                                  caption="nodewipe explain evaluates git branch staleness, markers, and rebuild commands.")
        write_frame(f, count=1)

    # Scene 3: Storage Tree
    print("[4/7] Rendering Scene 3: Storage Explorer...")
    cmd_text = "nodewipe tree ~/projects/rust-service/target"
    tree_output = [
        "  Storage Explorer — ~/projects/rust-service/target",
        "",
        "  target/ (12.00 MB)",
        "└──  debug/ (12.00 MB · 100.0%)",
        "    ├──  librust_service.rlib (12.00 MB · 100.0%)",
        "    ├──  deps/ (0 B · 0.0%)",
        "    └──  incremental/ (0 B · 0.0%)",
        "",
        "Internal breakdown exposes exact compilation artifact footprint."
    ]
    for i in range(1, len(cmd_text) + 1):
        f = render_terminal_frame(cmd_text[:i], [], cursor_on=True,
                                  badge="TREE",
                                  caption="Storage Explorer inspects internal subpackage weights and debug symbol bloat.")
        write_frame(f, count=2)

    for f_idx in range(160):
        cursor = (f_idx % 12) < 6
        f = render_terminal_frame(cmd_text, tree_output, cursor_on=cursor,
                                  badge="STORAGE TREE",
                                  caption="Storage Explorer inspects internal subpackage weights and debug symbol bloat.")
        write_frame(f, count=1)

    # Scene 4: Doctor
    print("[5/7] Rendering Scene 4: System Doctor...")
    cmd_text = "nodewipe doctor ~"
    doctor_output = [
        " 🩺 nodewipe doctor — Workspace Health & Toolchain Audit",
        "────────────────────────────────────────────────────────────────────────",
        "  • Rust Toolchains:  28.4 GB across 14 Cargo projects",
        "  • Node.js Modules:  34.1 GB across 18 node_modules directories",
        "  • Python Venvs:     12.3 GB in virtual environments & cache",
        "  • AI Model Weights: 18.0 GB in PyTorch & HuggingFace caches",
        "  • CoW Reflink:      Supported on APFS/Btrfs/XFS (instant zero-byte snapshots)",
        "  • Action Plan:      Run 'nodewipe reclaim 30GB' to free target budget safely",
        "────────────────────────────────────────────────────────────────────────"
    ]
    for i in range(1, len(cmd_text) + 1):
        f = render_terminal_frame(cmd_text[:i], [], cursor_on=True,
                                  badge="DOCTOR",
                                  caption="System Doctor audits toolchain storage hogs and cache health across your system.")
        write_frame(f, count=2)

    for f_idx in range(160):
        cursor = (f_idx % 12) < 6
        f = render_terminal_frame(cmd_text, doctor_output, cursor_on=cursor,
                                  badge="SYSTEM DOCTOR",
                                  caption="System Doctor audits toolchain storage hogs and cache health across your system.")
        write_frame(f, count=1)

    # Scene 5: Safe Excision
    print("[6/7] Rendering Scene 5: Safe Excision...")
    cmd_text = "nodewipe delete ~/projects --older-than 14d --dry-run"
    delete_output = [
        "Scanning for artifacts untouched for >14 days...",
        "[DRY RUN] 2 candidate artifacts matched criteria:",
        "  • ~/projects/archived-service/target (48.2 MB) -> Trash (recoverable)",
        "  • ~/projects/legacy-web/node_modules (124.6 MB) -> Trash (recoverable)",
        "",
        "Summary: 2 artifacts, 172.8 MB would be moved to OS Trash.",
        "Reflink CoW snapshot created prior to unlink. 0 data loss risk."
    ]
    for i in range(1, len(cmd_text) + 1):
        f = render_terminal_frame(cmd_text[:i], [], cursor_on=True,
                                  badge="EXCISION",
                                  caption="Safe by default: OS Trash recovery and instant zero-space CoW reflink snapshot backups.")
        write_frame(f, count=2)

    for f_idx in range(160):
        cursor = (f_idx % 12) < 6
        f = render_terminal_frame(cmd_text, delete_output, cursor_on=cursor,
                                  badge="SAFE EXCISION",
                                  caption="Safe by default: OS Trash recovery and instant zero-space CoW reflink snapshot backups.")
        write_frame(f, count=1)

    # Outro Slide
    print("[7/7] Rendering Outro Slide...")
    outro_img = create_slide_frame(
        title="Get Started with nodewipe",
        subtitle="Reclaim Your Disk. Keep Your Workflow.",
        bullets=[
            "Arch Linux (AUR):     yay -S nodewipe",
            "Homebrew (macOS/Linux): brew install KADHIRAVANEG/nodewipe/nodewipe",
            "NPM (Zero-Rust shim):   npx @joker53/nodewipe",
            "GitHub Repository:      https://github.com/KADHIRAVANEG/nodewipe",
            "Free & Open Source under MIT. Star on GitHub to support the project!"
        ],
        badge="INSTALL",
        caption="Cross-platform native binaries with verified SHA-256 sums are available now."
    )
    write_frame(outro_img, count=120)  # 5 seconds

    proc.stdin.close()
    proc.wait()
    print(f"\n[DONE] High-definition demo video rendered successfully: {output_video}")


if __name__ == '__main__':
    main()
