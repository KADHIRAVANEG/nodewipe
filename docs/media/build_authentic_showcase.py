#!/usr/bin/env python3
import os
import sys
import subprocess
from PIL import Image, ImageDraw, ImageFont, ImageSequence

W = 1920
H = 1080
FOOTER_H = 70
CONTENT_H = H - FOOTER_H
BG_DRACULA = (40, 42, 54) # #282a36
FPS = 24

FONT_PATH = "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"
if not os.path.exists(FONT_PATH):
    FONT_PATH = "/usr/share/fonts/noto/NotoSans-Bold.ttf"
FONT = ImageFont.truetype(FONT_PATH, 22)
FONT_LARGE = ImageFont.truetype(FONT_PATH, 32)
FONT_MED = ImageFont.truetype(FONT_PATH, 24)

def draw_footer(img: Image.Image, caption: str) -> Image.Image:
    """Draws a solid white footer bar with crisp black text across the bottom."""
    draw = ImageDraw.Draw(img)
    draw.rectangle([0, CONTENT_H, W, H], fill=(255, 255, 255))
    draw.text((36, CONTENT_H + 22), caption, fill=(0, 0, 0), font=FONT)
    return img

def create_get_started_slide() -> Image.Image:
    """Creates a clean, authentic terminal card for the Get Started summary."""
    slide = Image.new("RGB", (W, H), BG_DRACULA)
    draw = ImageDraw.Draw(slide)
    
    y = 110
    draw.text((120, y), "nodewipe v0.9.9 — Fast, Intelligent Workspace & Build Artifact Cleaner", fill=(80, 250, 123), font=FONT_LARGE)
    y += 60
    draw.text((120, y), "──────────────────────────────────────────────────────────────────────────────────────────", fill=(98, 114, 164), font=FONT_MED)
    
    y += 50
    draw.text((120, y), "Quick Installation:", fill=(139, 233, 253), font=FONT_LARGE)
    y += 55
    
    commands = [
        ("Cargo (Crates.io)", "cargo install nodewipe"),
        ("Homebrew (macOS/Linux)", "brew install kadhiravaneg/tap/nodewipe"),
        ("Arch Linux (AUR)", "yay -S nodewipe-bin"),
        ("npm (Zero-dependency)", "npm install -g nodewipe"),
        ("One-line Shell Script", "curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/install.sh | bash"),
    ]
    
    for label, cmd in commands:
        draw.text((140, y), f"• {label}:", fill=(248, 248, 242), font=FONT_MED)
        y += 36
        draw.text((180, y), f"$ {cmd}", fill=(241, 250, 140), font=FONT_MED)
        y += 50
        
    y += 20
    draw.text((120, y), "──────────────────────────────────────────────────────────────────────────────────────────", fill=(98, 114, 164), font=FONT_MED)
    y += 45
    draw.text((120, y), "GitHub Repository: https://github.com/KADHIRAVANEG/nodewipe  (MIT Licensed)", fill=(189, 147, 249), font=FONT_LARGE)
    
    return slide

def main():
    terminal_gif = sys.argv[1] if len(sys.argv) > 1 else "/tmp/clean_real_terminal_w130.gif"
    out_mp4 = sys.argv[2] if len(sys.argv) > 2 else "/home/joker53/Projects/npkill-rs/docs/media/nodewipe-demo.mp4"
    
    print(f"Loading terminal GIF from {terminal_gif}...")
    term_im = Image.open(terminal_gif)
    
    raw_frames = []
    durations = [] # in milliseconds
    for frame in ImageSequence.Iterator(term_im):
        raw_frames.append(frame.copy().convert("RGB"))
        dur = frame.info.get("duration", 100)
        if dur <= 0:
            dur = 100
        durations.append(dur)
        
    total_term_ms = sum(durations)
    print(f"Loaded {len(raw_frames)} terminal frames, total duration {total_term_ms/1000.0:.2f}s")
    
    tw, th = raw_frames[0].size
    scale = min((W - 80) / tw, (CONTENT_H - 40) / th)
    new_w, new_h = int(tw * scale), int(th * scale)
    paste_x = 40
    paste_y = 20
    print(f"Terminal frame size: {tw}x{th} -> scaled to {new_w}x{new_h} at ({paste_x}, {paste_y})")
    
    term_timeline = [
        (0, 7000, " [1/6 WORKSPACE SCAN]   Rayon multi-threaded parallel traversal scans multi-language project trees in milliseconds."),
        (7000, 13900, " [2/6 DEEP EXPLAIN]   Inspects project type, disk weight, safety risk level, and local rebuild recipe."),
        (13900, 20300, " [3/6 STORAGE TREE]   Visualizes internal subpackage weights and compilation artifact footprint."),
        (20300, 26700, " [4/6 DRY-RUN SAFETY]   Simulates exact files and gigabytes to be reclaimed before touching disk."),
        (26700, 33600, " [5/6 SYSTEM DOCTOR]   Scans global developer caches and surfaces inactive bloat across ecosystems."),
        (33600, total_term_ms + 1000, " [6/6 EXTENSIBLE RULES]   Built-in engine with 39+ language ecosystems and custom regex pattern support."),
    ]
    
    def get_term_caption(elapsed_ms: float) -> str:
        for start, end, cap in term_timeline:
            if start <= elapsed_ms < end:
                return cap
        return term_timeline[-1][2]

    scaled_frames = [f.resize((new_w, new_h), Image.Resampling.LANCZOS) for f in raw_frames]

    ffmpeg_cmd = [
        "ffmpeg", "-y",
        "-f", "rawvideo",
        "-vcodec", "rawvideo",
        "-s", f"{W}x{H}",
        "-pix_fmt", "rgb24",
        "-r", str(FPS),
        "-i", "-",
        "-c:v", "libx264",
        "-preset", "fast",
        "-crf", "18",
        "-pix_fmt", "yuv420p",
        "-movflags", "+faststart",
        out_mp4
    ]
    
    print(f"Starting ffmpeg encoding to {out_mp4}...")
    proc = subprocess.Popen(ffmpeg_cmd, stdin=subprocess.PIPE)
    
    # 1. Output terminal frames
    current_ms = 0.0
    frame_idx = 0
    time_accumulator = 0.0
    frame_interval_ms = 1000.0 / FPS
    
    while current_ms < total_term_ms and frame_idx < len(scaled_frames):
        target_time = current_ms
        while frame_idx < len(scaled_frames) - 1 and time_accumulator + durations[frame_idx] < target_time:
            time_accumulator += durations[frame_idx]
            frame_idx += 1
            
        scaled = scaled_frames[frame_idx]
        
        canvas = Image.new("RGB", (W, H), BG_DRACULA)
        canvas.paste(scaled, (paste_x, paste_y))
        
        cap = get_term_caption(current_ms)
        draw_footer(canvas, cap)
        
        proc.stdin.write(canvas.tobytes())
        current_ms += frame_interval_ms

    print("Terminal section completed. Adding Interactive TUI slides...")
    # 2. Interactive TUI - Main View (8 seconds)
    tui_main_src = Image.open("docs/screenshots/tui-main.png").convert("RGB")
    tui_main_scaled = tui_main_src.resize((W, CONTENT_H), Image.Resampling.LANCZOS)
    canvas = Image.new("RGB", (W, H), (0, 0, 0))
    canvas.paste(tui_main_scaled, (0, 0))
    draw_footer(canvas, " [INTERACTIVE TUI]   Fast keyboard navigation (↑/↓), spacebar multi-selection, and real-time disk recalculation.")
    tui_main_bytes = canvas.tobytes()
    for _ in range(int(FPS * 8.0)):
        proc.stdin.write(tui_main_bytes)
        
    # 3. Interactive TUI - Dry-run View (8 seconds)
    tui_dryrun_src = Image.open("docs/screenshots/tui-dryrun.png").convert("RGB")
    tui_dryrun_scaled = tui_dryrun_src.resize((W, CONTENT_H), Image.Resampling.LANCZOS)
    canvas = Image.new("RGB", (W, H), (0, 0, 0))
    canvas.paste(tui_dryrun_scaled, (0, 0))
    draw_footer(canvas, " [TUI DRY-RUN PREVIEW]   Instant safety confirmation verifies every selected directory before cleanup begins.")
    tui_dryrun_bytes = canvas.tobytes()
    for _ in range(int(FPS * 8.0)):
        proc.stdin.write(tui_dryrun_bytes)

    print("Adding Desktop GUI slide...")
    # 4. Desktop GUI (12 seconds)
    gui_src = Image.open("docs/screenshots/gui-overview.png").convert("RGB")
    gui_scaled = gui_src.resize((W, CONTENT_H), Image.Resampling.LANCZOS)
    canvas = Image.new("RGB", (W, H), (0, 0, 0))
    canvas.paste(gui_scaled, (0, 0))
    draw_footer(canvas, " [DESKTOP GUI]   Native Tauri v2 desktop application sharing the exact same compiled Rust engine.")
    gui_bytes = canvas.tobytes()
    for _ in range(int(FPS * 12.0)):
        proc.stdin.write(gui_bytes)

    print("Adding Get Started summary slide...")
    # 5. Get Started Summary (8 seconds)
    summary_slide = create_get_started_slide()
    draw_footer(summary_slide, " [GET STARTED]   Open source under MIT License. Star the repository: github.com/KADHIRAVANEG/nodewipe")
    summary_bytes = summary_slide.tobytes()
    for _ in range(int(FPS * 8.0)):
        proc.stdin.write(summary_bytes)

    proc.stdin.close()
    proc.wait()
    print(f"Successfully generated {out_mp4}!")

if __name__ == "__main__":
    main()
