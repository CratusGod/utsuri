"""End-to-end test on a real Windows desktop.

Every scenario: real mouse drag of a file (from File Explorer or the Desktop) → press Shift →
drop on a slice → (drive the tool window) → verify the output file on disk.
Writes e2e_out/results.json, results.md, log.txt, screenshots and a screen recording."""
from __future__ import annotations

import json
import os
import shutil
import time
import traceback
import zipfile
from pathlib import Path

import pyautogui
from PIL import Image, ImageDraw

import winkit as wk
import utsuri.convert  # noqa: F401  (registers HEIC/AVIF support for the verifiers)

OUT = wk.ROOT / "e2e_out"
OUT.mkdir(exist_ok=True)
(OUT / "shots").mkdir(exist_ok=True)
EXP = Path(r"C:\UtsuriTest")
DESK = wk.desktop_dir()
STASH = Path(r"C:\UtsuriTest_outputs")
RESULTS: list[dict] = []
# UTSURI_E2E_LOCAL=1: running on someone's own PC. Keep their screen settings, and never delete a
# Desktop file that was there before the run. UTSURI_E2E_ONLY=a,b runs only scenarios whose name
# contains one of those words.
LOCAL = os.environ.get("UTSURI_E2E_LOCAL") == "1"
ONLY = [w.strip().lower() for w in os.environ.get("UTSURI_E2E_ONLY", "").split(",") if w.strip()]
DESK_BEFORE: set[str] = set()
VI_TEXT = "Bản nháp – Tài liệu mật"


# ------------------------------------------------------------------ sample files
def make_photo(path: Path, size=(2400, 1600), gps=True):
    import random
    random.seed(3)
    im = Image.new("RGB", size)
    d = ImageDraw.Draw(im)
    for y in range(size[1]):
        t = y / size[1]
        d.line([(0, y), (size[0], y)], fill=(int(255 - 120 * t), int(170 - 90 * t), int(90 + 110 * t)))
    for _ in range(250):
        x, y = random.randrange(size[0]), random.randrange(size[1])
        r = random.randrange(10, 90)
        d.ellipse((x, y, x + r, y + r), fill=tuple(random.randrange(256) for _ in range(3)))
    ex = Image.Exif()
    ex[0x010F] = "Apple"
    ex[0x0110] = "iPhone 15 Pro"
    if gps:
        ex[0x8825] = {1: "N", 2: (24.0, 33.0, 18.0), 3: "W", 4: (81.0, 47.0, 0.0)}
    im.save(path, quality=95, exif=ex)


def make_pdf(path: Path, img: Path, pages=3, title="Report"):
    import pymupdf
    doc = pymupdf.open()
    for i in range(pages):
        pg = doc.new_page()
        pg.insert_text((72, 80), f"{title} — page {i + 1}", fontsize=22)
        for j in range(12):
            pg.insert_text((72, 120 + j * 18), "Lorem ipsum dolor sit amet, consectetur adipiscing.", fontsize=11)
        pg.insert_image(pymupdf.Rect(72, 360, 420, 600), filename=str(img))
    doc.set_metadata({"author": "Jimmy", "title": title})
    doc.save(path)


def make_files(folder: Path, prefix=""):
    folder.mkdir(exist_ok=True)
    make_photo(folder / f"{prefix}Photo.jpg")
    make_photo(folder / f"{prefix}Second.jpg", size=(1200, 900), gps=False)
    Image.open(folder / f"{prefix}Photo.jpg").resize((800, 533)).save(folder / f"{prefix}Graphic.png")
    make_pdf(folder / f"{prefix}Report.pdf", folder / f"{prefix}Second.jpg")
    make_pdf(folder / f"{prefix}Appendix.pdf", folder / f"{prefix}Second.jpg", pages=2, title="Appendix")
    from docx import Document
    d = Document()
    d.add_heading("Letter", 1)
    d.add_paragraph("Hello from the Utsuri end-to-end test.")
    d.save(folder / f"{prefix}Letter.docx")
    (folder / f"{prefix}Notes.txt").write_text("Shopping list\n- coffee\n- limes\n", encoding="utf-8")
    with zipfile.ZipFile(folder / f"{prefix}Bundle.zip", "w") as z:
        z.writestr("Bundle/readme.txt", "inside the archive\n")
        z.write(folder / f"{prefix}Graphic.png", "Bundle/Graphic.png")
    make_sign(folder / f"{prefix}Sign.png")
    make_clip(folder / f"{prefix}Clip.mp4")
    make_photo(folder / f"{prefix}Ảnh chụp.jpg", size=(900, 600), gps=False)  # a Vietnamese file name


def make_sign(path: Path):
    """Big dark words on white: something Tesseract reads reliably. Not too wide, so Explorer's
    thumbnail fills the icon and a press at the icon's centre lands on the file."""
    from PIL import ImageFont
    im = Image.new("RGB", (1000, 800), "white")
    font = ImageFont.load_default(size=130)
    d = ImageDraw.Draw(im)
    d.text((70, 220), "UTSURI", fill="black", font=font)
    d.text((70, 430), "2026", fill="black", font=font)
    im.save(path)


def make_clip(path: Path):
    """3 s of test pattern with a tone, so every video target (GIF, WEBM, MP3) has input.
    The Linux and Mac runs share make_files: without any ffmpeg there, no clip (they don't use it)."""
    import subprocess
    try:
        import imageio_ffmpeg
        exe = imageio_ffmpeg.get_ffmpeg_exe()
    except ImportError:
        exe = shutil.which("ffmpeg")
    if exe is None:
        return
    subprocess.run([exe, "-loglevel", "error", "-y",
                    "-f", "lavfi", "-i", "testsrc=duration=3:size=640x360:rate=24",
                    "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
                    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", str(path)],
                   check=True, capture_output=True)


def snapshot(folder: Path) -> set[str]:
    return {str(p.relative_to(folder)) for p in folder.rglob("*")}


def new_files(folder: Path, before: set[str], timeout=15) -> list[Path]:
    """New files (ignoring in-progress '.~*.part'), once their sizes stop changing."""
    end = time.time() + timeout

    def current():
        now = snapshot(folder) - before
        return {n: (folder / n).stat().st_size for n in now   # not Utsuri's in-progress temp files
                if (folder / n).is_file() and not Path(n).name.startswith((".~", ".utsuri-"))}
    while time.time() < end:
        a = current()
        if a:
            time.sleep(0.8)
            b = current()
            if a == b:
                return sorted(folder / n for n in b)
        time.sleep(0.25)
    return []


def stash(folder: Path, before: set[str]):
    """Move outputs out of the way so the folder only holds the sources again."""
    STASH.mkdir(exist_ok=True)
    for n in sorted(snapshot(folder) - before, key=len, reverse=True):
        p = folder / n
        try:
            if p.is_file():
                dest = STASH / f"{int(time.time() * 1000)}_{p.name}"
                shutil.move(str(p), dest)
            elif p.is_dir():
                p.rmdir()
        except Exception:
            pass


SOURCES = ["Photo.jpg", "Second.jpg", "Graphic.png", "Report.pdf", "Appendix.pdf", "Letter.docx", "Notes.txt",
           "Bundle.zip", "Sign.png", "Clip.mp4", "Ảnh chụp.jpg"]


def restore_sources(folder: Path):
    """If an earlier failure let Windows move a file away (Shift-drop without the menu = move),
    put it back so one failure can't cascade into the rest of the run."""
    prefix = "" if folder == EXP else "D-"
    missing = [n for n in SOURCES if not (folder / (prefix + n)).exists()]
    if missing:
        wk.log("restoring sources:", missing)
        for n in missing:
            stray = DESK / (prefix + n) if folder == EXP else None
            if stray is not None and stray.exists() and stray.name not in DESK_BEFORE:
                stray.unlink()
        make_files(folder, prefix)
        time.sleep(1.0)
    # stray copies of Explorer files dropped on the desktop (never one that was there before the run)
    for n in SOURCES:
        if (DESK / n).exists() and n not in DESK_BEFORE:
            (DESK / n).unlink()


# ------------------------------------------------------------------ verification helpers
def img_ok(p: Path, fmt=None):
    with Image.open(p) as im:
        im.load()
        if fmt:
            assert im.format == fmt, f"{p.name}: format {im.format} != {fmt}"
        return im.size


def pdf_pages(p: Path) -> int:
    import pymupdf
    with pymupdf.open(p) as d:
        return d.page_count


# ------------------------------------------------------------------ menu layout (from the app itself)
def ring_index(paths: list[str], ring: str, label: str) -> tuple[int, int, int]:
    from utsuri.app import UtsuriApp
    rings = UtsuriApp.build_rings(None, paths)
    names = list(rings)
    items = rings[ring]
    labels = [i.label for i in items]
    return names.index(ring), labels.index(label), len(items)


# ------------------------------------------------------------------ scenario runner
def safe(name: str) -> str:
    return "".join(c if c.isalnum() or c in "-_ ." else "_" for c in name).replace(" ", "_")


def run(name: str, where: str, files: list[str], ring: str, label: str, verify, tool=None, select_extra=None,
        expect_files=True, after=None):
    """expect_files=False: the slice makes no file (Copy), so verify gets []. after(outs) runs once the
    outputs are checked, e.g. to press Undo, and may add to the note."""
    if ONLY and not any(w in name.lower() for w in ONLY):
        return
    folder = EXP if where == "explorer" else DESK
    srcs = [str(folder / f) for f in files]
    restore_sources(folder)
    before = snapshot(folder)
    t0 = time.time()
    status, msg = "PASS", ""
    try:
        # Explorer hands over the dragged file (the last one clicked) first, and the menu's
        # rings follow the first file's kind
        order = [srcs[-1], *srcs[:-1]] if select_extra else srcs
        ring_i, idx, n = ring_index(order, ring, label)
        container = EX if where == "explorer" else wk.desktop_view()
        if where == "desktop":
            pyautogui.hotkey("win", "d") if not DESKTOP_SHOWN[0] else None
            DESKTOP_SHOWN[0] = True
        else:
            if DESKTOP_SHOWN[0]:
                EX.set_focus()
                DESKTOP_SHOWN[0] = False
                time.sleep(0.6)
        # select extra files first (multi-select), then drag the last one
        if select_extra:
            first = wk.item_center(container, files[0])
            wk.glide(*first, 0.4)
            pyautogui.click()
            for f in files[1:]:
                c = wk.item_center(container, f)
                wk.glide(*c, 0.35)
                pyautogui.keyDown("ctrl")
                pyautogui.click()
                pyautogui.keyUp("ctrl")
            src_xy = wk.item_center(container, files[-1])
        else:
            src_xy = wk.item_center(container, files[0])
        sw, sh = pyautogui.size()
        dy = 260 if src_xy[1] < 300 else (60 if src_xy[1] < sh - 400 else -260)
        nudge = (min(640, sw - src_xy[0] - 260), dy)
        c = wk.drag_open_menu(src_xy, nudge)
        for _ in range(ring_i):  # each Shift tap moves to the next ring
            wk.switch_ring()
        target = wk.petal_xy(c, n, idx)
        wk.glide(*target, 0.4)
        time.sleep(0.35)
        wk.shot(OUT / "shots" / f"{len(RESULTS) + 1:02d}_{safe(name)}_menu.png")
        wk.drop()
        if tool:
            tool()
        if expect_files:
            outs = new_files(folder, before)
            if not outs:
                raise AssertionError("no output file appeared")
        else:
            time.sleep(2.5)
            outs = []
        note = verify(outs, srcs)
        msg = note or ", ".join(o.name for o in outs[:3])
        if after:
            msg = f"{msg} · {after(outs, folder, before)}"
    except Exception as e:
        status = "FAIL"
        msg = f"{type(e).__name__}: {e}"
        wk.log(traceback.format_exc())
        wk.shot(OUT / "shots" / f"{len(RESULTS) + 1:02d}_FAIL_{safe(name)}.png")
        pyautogui.mouseUp()
        pyautogui.keyUp("shift")
        pyautogui.press("esc")
        time.sleep(0.5)
        # close any stray Utsuri window
        try:
            for w in wk.uia().windows(title_re=".*— Utsuri"):
                w.close()
        except Exception:
            pass
    dt = time.time() - t0
    RESULTS.append({"name": name, "where": where, "status": status, "detail": msg, "seconds": round(dt, 1),
                    "t": round(time.time() - wk.T0, 1)})
    wk.log(f"{status:4} {where:8} {name:34} {dt:5.1f}s  {msg}")
    stash(folder, before)
    time.sleep(0.6)


# ------------------------------------------------------------------ tool drivers
def tool_win(title_re):
    return wk.find_window(title_re, 12)


def drive_compress():
    w = tool_win("Compress.*")
    time.sleep(0.5)
    wk.click_title(w, "Target size")
    wk.click_title(w, "200 KB", 0.8)
    btns = [b for b in w.descendants(control_type="Button") if b.window_text().startswith("Compress")]
    r = btns[-1].rectangle()
    wk.glide((r.left + r.right) // 2, (r.top + r.bottom) // 2)
    pyautogui.click()


def drive_resize():
    w = tool_win("Resize.*")
    wk.click_title(w, "50%", 0.5)
    btns = [b for b in w.descendants(control_type="Button") if b.window_text().startswith("Resize")]
    r = btns[-1].rectangle()
    wk.glide((r.left + r.right) // 2, (r.top + r.bottom) // 2)
    pyautogui.click()


def drive_crop():
    w = tool_win("Crop Image.*")
    time.sleep(0.4)
    wk.click_title(w, "1:1", 0.6)
    wk.click_title(w, "Apply")


def drive_edit():
    w = tool_win("Edit.*")
    time.sleep(0.4)
    wk.click_title(w, "✦ Auto", 0.6)
    wk.click_title(w, "⟳", 0.6)
    wk.click_title(w, "Save")


def _canvas(w):
    r = wk.image_area(w)
    if r is None:
        wr = w.rectangle()
        return (wr.left + 60, wr.top + 80, wr.right - 60, wr.top + int(wr.height() * 0.6))
    return (r.left, r.top, r.right, r.bottom)


def drive_annotate():
    w = tool_win("Annotate.*")
    time.sleep(0.6)
    x0, y0, x1, y1 = _canvas(w)
    cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
    wk.glide(cx - 150, cy + 80)
    pyautogui.mouseDown()
    wk.glide(cx + 120, cy - 60, 0.5)
    pyautogui.mouseUp()
    wk.click_title(w, "① Step", 0.3)
    wk.glide(cx + 160, cy - 80)
    pyautogui.click()
    time.sleep(0.4)
    wk.click_title(w, "Save")


def drive_redact():
    w = tool_win("Redact.*")
    time.sleep(0.6)
    x0, y0, x1, y1 = _canvas(w)
    cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
    wk.glide(cx - 120, cy - 60)
    pyautogui.mouseDown()
    wk.glide(cx + 120, cy + 60, 0.5)
    pyautogui.mouseUp()
    time.sleep(0.5)
    wk.click_title(w, "Save")


def drive_addbg():
    w = tool_win("Add BG.*")
    time.sleep(0.7)
    wk.click_title(w, "1:1", 0.6)
    wk.click_title(w, "Save with Background")


def drive_meta_location():
    w = tool_win("Metadata.*")
    time.sleep(0.5)
    wk.click_title(w, "Remove location only")


def drive_meta_all():
    w = tool_win("Metadata.*")
    time.sleep(0.5)
    wk.click_title(w, "Remove all metadata")


def drive_protect():
    w = tool_win("Password-protect.*")
    time.sleep(0.4)
    edits = w.descendants(control_type="Edit")
    for e in edits[:2]:
        r = e.rectangle()
        wk.glide((r.left + r.right) // 2, (r.top + r.bottom) // 2, 0.3)
        pyautogui.click()
        pyautogui.typewrite("keywest2026", interval=0.02)
    time.sleep(0.3)
    wk.click_title(w, "Protect")


# ------------------------------------------------------------------ verifiers
def v_format(fmt, ext):
    def v(outs, srcs):
        o = outs[0]
        assert o.suffix.lower() == ext, f"extension {o.suffix}"
        if fmt == "PDF":
            assert pdf_pages(o) >= 1
            return f"{o.name} · {pdf_pages(o)} page(s)"
        if fmt in ("DOCX",):
            from docx import Document
            assert Document(o).paragraphs
            return o.name
        if fmt == "TXT":
            assert o.read_text(encoding="utf-8").strip()
            return o.name
        size = img_ok(o, None if fmt in ("HEIC", "AVIF") else fmt)
        return f"{o.name} · {size[0]}×{size[1]}"
    return v


def v_pages_to_images(n, ext):
    def v(outs, srcs):
        imgs = [o for o in outs if o.suffix.lower() == ext]
        assert len(imgs) == n, f"{len(imgs)} images, expected {n}"
        for o in imgs:
            img_ok(o)
        return f"{n} × {ext}"
    return v


def v_compress(limit):
    def v(outs, srcs):
        o = outs[0]
        s0, s1 = Path(srcs[0]).stat().st_size, o.stat().st_size
        assert s1 <= limit, f"{s1} bytes > {limit}"
        img_ok(o)
        return f"{s0 // 1024} KB → {s1 // 1024} KB ({(1 - s1 / s0) * 100:.0f}% smaller)"
    return v


def v_dims(fn):
    def v(outs, srcs):
        with Image.open(srcs[0]) as a:
            sa = a.size
        sb = img_ok(outs[0])
        assert fn(sa, sb), f"{sa} → {sb}"
        return f"{sa[0]}×{sa[1]} → {sb[0]}×{sb[1]}"
    return v


def v_exists(outs, srcs):
    img_ok(outs[0])
    return outs[0].name


def v_redact(outs, srcs):
    with Image.open(outs[0]) as im:
        im = im.convert("RGB")
        w, h = im.size
        px = im.getpixel((w // 2, h // 2))
    assert sum(px) < 40, f"centre pixel {px} not blacked out"
    return f"{outs[0].name} · centre blacked out"


def v_no_gps_keep_camera(outs, srcs):
    from utsuri import imaging
    s = imaging.metadata_summary(outs[0])
    assert "Location" not in s, "GPS still present"
    assert "Camera" in s, "camera info was removed"
    return f"GPS removed, kept {s['Camera']}"


def v_no_meta(outs, srcs):
    from utsuri import imaging
    n, gps = imaging.count_metadata(outs[0])
    assert n == 0 and not gps, f"{n} fields remain"
    return "0 metadata fields left"


def v_pdf_n(n):
    def v(outs, srcs):
        pdfs = [o for o in outs if o.suffix.lower() == ".pdf"]
        assert len(pdfs) == n, f"{len(pdfs)} PDFs"
        return f"{n} PDFs"
    return v


def v_merged(outs, srcs):
    total = sum(pdf_pages(Path(s)) for s in srcs)
    got = pdf_pages(outs[0])
    assert got == total, f"{got} pages, expected {total}"
    return f"{got} pages"


def v_rotated(outs, srcs):
    import pymupdf
    with pymupdf.open(outs[0]) as d:
        assert all(p.rotation == 90 for p in d)
    return "all pages at 90°"


def v_protected(outs, srcs):
    import pymupdf
    d = pymupdf.open(outs[0])
    assert d.needs_pass and d.authenticate("keywest2026")
    return "AES-256, password works"


def v_pdf_meta(outs, srcs):
    import pymupdf
    with pymupdf.open(outs[0]) as d:
        assert not (d.metadata or {}).get("author")
    return "author/title removed"


def v_pdf_compress(outs, srcs):
    s0, s1 = Path(srcs[0]).stat().st_size, outs[0].stat().st_size
    assert pdf_pages(outs[0]) == pdf_pages(Path(srcs[0]))
    return f"{s0 // 1024} KB → {s1 // 1024} KB"


def v_extract(outs, srcs):
    assert outs and all(o.parent.name.endswith("images") for o in outs)
    for o in outs:
        img_ok(o)
    return f"{len(outs)} image(s) in {outs[0].parent.name}/"


def v_combined(outs, srcs):
    assert pdf_pages(outs[0]) == len(srcs)
    return f"{len(srcs)} images → {pdf_pages(outs[0])}-page PDF"


def drive_watermark_vi():
    w = tool_win("Watermark or stamp.*")
    time.sleep(0.6)
    box = next(e for e in w.descendants(control_type="Edit") if e.get_value() == "DRAFT")
    box.set_edit_text(VI_TEXT)  # UIA ValuePattern: typing can't produce Vietnamese letters
    time.sleep(0.8)              # the preview refreshes after 150 ms
    wk.shot(OUT / "shots" / f"{len(RESULTS) + 1:02d}_watermark_preview.png")
    wk.click_title(w, "Apply")


def v_watermark_vi(outs, srcs):
    """pdfkit draws the watermark as a faded image (viewers ignore PDF text opacity), so: one
    more image on every page, and every letter drawn by a font that really has it."""
    import pymupdf
    from utsuri import pdfkit
    pdfs = [o for o in outs if o.suffix.lower() == ".pdf"]
    assert pdfs, "no PDF"
    with pymupdf.open(srcs[0]) as a, pymupdf.open(pdfs[0]) as b:
        added = [len(pb.get_images()) - len(pa.get_images()) for pa, pb in zip(a, b)]
        b[0].get_pixmap(dpi=60).save(OUT / "shots" / f"{len(RESULTS) + 1:02d}_watermark_page1.png")
    assert added and all(n == 1 for n in added), f"images added per page: {added}"
    # Helvetica has á/à/–; letters like ả/ệ/ậ need the system font fallback
    fonts = sorted({f.name for t, f in pdfkit._runs(VI_TEXT, 72) if any(ord(c) > 0xFF and c != "–" for c in t)})
    assert fonts and "Helvetica" not in fonts, f"Vietnamese letters drawn with {fonts}"
    return f"{pdfs[0].name} · on all {len(added)} pages · Vietnamese letters from {', '.join(fonts)}"


def v_zip(outs, srcs):
    z = [o for o in outs if o.suffix.lower() == ".zip"]
    assert z, "no zip"
    with zipfile.ZipFile(z[0]) as f:
        names = f.namelist()
    assert Path(srcs[0]).name in names, names
    return f"{z[0].name} holds {', '.join(names)}"


def v_tar(outs, srcs):
    import tarfile
    t = [o for o in outs if o.name.endswith(".tar.gz")]
    assert t, "no .tar.gz"
    with tarfile.open(t[0]) as f:
        names = f.getnames()
    assert Path(srcs[0]).name in names, names
    return f"{t[0].name} holds {', '.join(names)}"


def v_long_side(n):
    def v(outs, srcs):
        size = img_ok(outs[0])
        assert max(size) == n, f"{size}"
        return f"{outs[0].name} · {size[0]}×{size[1]}"
    return v


def v_gray_pdf(outs, srcs):
    import pymupdf
    with pymupdf.open(outs[0]) as d:
        pix = d[0].get_pixmap(dpi=40)
    assert pix.n < 3 or _is_gray(pix), "page still has colour"
    return f"{outs[0].name} · grayscale"


def _is_gray(pix) -> bool:
    s = pix.samples
    return all(abs(s[i] - s[i + 1]) < 8 and abs(s[i + 1] - s[i + 2]) < 8 for i in range(0, len(s), pix.n * 97))


def clipboard_text() -> str:
    import tkinter
    r = tkinter.Tk()
    r.withdraw()
    try:
        return r.clipboard_get()
    finally:
        r.destroy()


def v_clipboard_image(outs, srcs):
    from PIL import ImageGrab
    for _ in range(10):
        im = ImageGrab.grabclipboard()
        if isinstance(im, Image.Image):
            break
        time.sleep(0.5)
    assert isinstance(im, Image.Image), f"clipboard holds {type(im).__name__}"
    with Image.open(srcs[0]) as a:
        assert im.size == a.size, f"{im.size} != {a.size}"
    return f"clipboard image {im.size[0]}×{im.size[1]}"


def v_ocr(outs, srcs):
    txt = [o for o in outs if o.suffix.lower() == ".txt"]
    assert txt, "no text file"
    got = txt[0].read_text(encoding="utf-8")
    assert "UTSURI" in got.upper(), f"read {got!r}"
    clip = clipboard_text()
    assert "UTSURI" in clip.upper(), f"clipboard {clip!r}"
    return f"{txt[0].name}: {got.strip()!r}, also on the clipboard"


def v_gif(outs, srcs):
    with Image.open(outs[0]) as im:
        assert im.format == "GIF" and getattr(im, "n_frames", 1) > 5, f"{im.format} {getattr(im, 'n_frames', 1)}"
        return f"{outs[0].name} · {im.n_frames} frames · {im.size[0]}×{im.size[1]}"


def v_media(ext, codec_type):
    def v(outs, srcs):
        import subprocess
        import imageio_ffmpeg
        o = outs[0]
        assert o.suffix.lower() == ext, o.name
        r = subprocess.run([imageio_ffmpeg.get_ffmpeg_exe(), "-hide_banner", "-i", str(o)],
                           capture_output=True, text=True, errors="replace")
        assert f"{codec_type}:" in r.stderr, r.stderr[-300:]
        return f"{o.name} · {o.stat().st_size // 1024} KB · has {codec_type.lower()}"
    return v


def v_extracted(outs, srcs):
    names = sorted(o.name for o in outs)
    assert names == ["Graphic.png", "readme.txt"], names
    return f"extracted {', '.join(names)} into {outs[0].parent.name}/"


def v_zip_many(names):
    def v(outs, srcs):
        z = [o for o in outs if o.suffix.lower() == ".zip"]
        assert z, "no zip"
        with zipfile.ZipFile(z[0]) as f:
            got = sorted(f.namelist())
        assert got == sorted(names), got
        return f"{z[0].name} holds {', '.join(got)}"
    return v


def undo_file_and_check(outs, folder, before):
    """Press Undo on the newest result card; the new file must go to the Recycle Bin."""
    end = time.time() + 6
    cards = []
    while time.time() < end and not cards:
        cards = wk.toast_rects()
        time.sleep(0.3)
    assert cards, "no result card on screen"
    time.sleep(0.6)
    wk.glide(*wk.toast_button_xy(cards[0], "Undo"), 0.4)
    time.sleep(0.2)
    pyautogui.click()
    end = time.time() + 8
    while time.time() < end and any(o.exists() for o in outs):
        time.sleep(0.3)
    assert not any(o.exists() for o in outs), f"{[o.name for o in outs if o.exists()]} still there after Undo"
    return f"Undo removed {', '.join(o.name for o in outs)}"


def undo_and_check(outs, folder, before):
    """Press Undo on the newest result card; the extracted folder must go to the Recycle Bin."""
    top = folder / outs[0].relative_to(folder).parts[0]
    end = time.time() + 6
    cards = []
    while time.time() < end and not cards:
        cards = wk.toast_rects()
        time.sleep(0.3)
    assert cards, "no result card on screen"
    time.sleep(0.6)  # the card turns from spinner to result
    xy = wk.toast_button_xy(cards[0], "Undo")
    wk.glide(*xy, 0.4)
    time.sleep(0.2)
    wk.shot(OUT / "shots" / f"{len(RESULTS) + 1:02d}_undo_card.png")
    pyautogui.click()
    end = time.time() + 8
    while time.time() < end and top.exists():
        time.sleep(0.3)
    assert not top.exists(), f"{top.name} still there after Undo"
    return f"Undo removed {top.name}/"


# ------------------------------------------------------------------ colour themes
def set_theme_setting(name: str):
    import subprocess
    subprocess.run(["reg", "add", r"HKCU\Software\Utsuri\Utsuri", "/v", "theme", "/t", "REG_SZ", "/d", name, "/f"],
                   capture_output=True)


def theme_tour(app):
    """Each theme: restart Utsuri with it, open the menu over a real drag, screenshot it.
    Passes when every theme draws a different menu and the app logged no error."""
    from utsuri import themes
    if ONLY and not any(w in "theme tour" for w in ONLY):
        return app
    t0 = time.time()
    looks, status, msg = {}, "PASS", ""
    log_before = len(_app_log())
    try:
        for name in themes.names():
            set_theme_setting(name)
            wk.stop_app(app)
            app = wk.start_app()
            time.sleep(4.5)
            EX.set_focus()
            time.sleep(0.5)
            src = wk.item_center(EX, "Photo.jpg")
            c = wk.drag_open_menu(src, (min(640, pyautogui.size()[0] - src[0] - 260), 60))
            time.sleep(0.9)  # the liquid glass renders while the menu pops in
            box = (c[0] - 200, c[1] - 200, c[0] + 200, c[1] + 200)
            from PIL import ImageGrab
            im = ImageGrab.grab(bbox=box)
            im.save(OUT / "shots" / f"theme_{safe(name)}.png")
            pyautogui.press("esc")
            pyautogui.mouseUp()
            pyautogui.keyUp("shift")
            time.sleep(0.8)
            looks[name] = im.convert("RGB").resize((24, 24))
        # Every theme must clearly change the menu; themes whose palettes are alike
        # (the green ones) may look close to each other, but never the same.
        names = list(looks)
        from_default = {n: _image_distance(looks[themes.DEFAULT], looks[n]) for n in names[1:]}
        weak = [n for n, d in from_default.items() if d < 2.0]
        assert not weak, f"barely differs from Default: {weak}"
        pairs = {(a, b): _image_distance(looks[a], looks[b]) for i, a in enumerate(names) for b in names[i + 1:]}
        same = [p for p, d in pairs.items() if d < 0.3]
        assert not same, f"these look identical: {same}"
        errors = [ln for ln in _app_log()[log_before:] if "Unhandled" in ln or "Traceback" in ln]
        assert not errors, errors[0]
        (a, b), d = min(pairs.items(), key=lambda kv: kv[1])
        msg = f"{len(looks)} themes, all distinct (closest: {a} / {b}, {d:.1f}) · shots/theme_*.png"
    except Exception as e:
        status, msg = "FAIL", f"{type(e).__name__}: {e}"
        wk.log(traceback.format_exc())
        pyautogui.mouseUp()
        pyautogui.keyUp("shift")
    finally:
        set_theme_setting(themes.DEFAULT)
    dt = time.time() - t0
    RESULTS.append({"name": "Colour themes", "where": "explorer", "status": status, "detail": msg,
                    "seconds": round(dt, 1), "t": round(time.time() - wk.T0, 1)})
    wk.log(f"{status:4} explorer {'Colour themes':34} {dt:5.1f}s  {msg}")
    return app


def _app_log() -> list[str]:
    p = Path(os.environ.get("LOCALAPPDATA", "")) / "Utsuri" / "log.txt"
    try:
        return p.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []


def _image_distance(a, b) -> float:
    """Mean absolute difference per channel (0-255) of two small RGB images."""
    pa, pb = a.getdata(), b.getdata()
    return sum(abs(x - y) for p, q in zip(pa, pb) for x, y in zip(p, q)) / (len(pa) * 3)


# ------------------------------------------------------------------ IPC / Send To path
def run_ipc(name: str, file: str, label: str, verify):
    """Launch a second copy with a file (what 'Send to' / right-click does) → menu → click."""
    if ONLY and not any(w in name.lower() for w in ONLY):
        return
    import subprocess
    import sys
    folder = EXP
    before = snapshot(folder)
    t0 = time.time()
    status, msg = "PASS", ""
    try:
        sw, sh = pyautogui.size()
        wk.glide(int(sw * 0.62), int(sh * 0.45), 0.4)
        c = wk.menu_center(pyautogui.position())
        pyw = Path(sys.executable).with_name("pythonw.exe")
        env = dict(__import__("os").environ, UTSURI_RECORDING="1")
        subprocess.Popen([str(pyw), "-m", "utsuri", str(folder / file)], cwd=str(wk.ROOT), env=env)
        time.sleep(2.5)  # instant hand-off to the running copy, menu opens
        _, idx, n = ring_index([str(folder / file)], "Convert", label)
        wk.glide(*wk.petal_xy(c, n, idx), 0.4)
        time.sleep(0.3)
        wk.shot(OUT / "shots" / f"{len(RESULTS) + 1:02d}_{safe(name)}_menu.png")
        pyautogui.click()
        outs = new_files(folder, before)
        if not outs:
            raise AssertionError("no output file appeared")
        msg = verify(outs, [str(folder / file)])
    except Exception as e:
        status, msg = "FAIL", f"{type(e).__name__}: {e}"
        wk.log(traceback.format_exc())
        pyautogui.press("esc")
    dt = time.time() - t0
    RESULTS.append({"name": name, "where": "send-to", "status": status, "detail": msg, "seconds": round(dt, 1),
                    "t": round(time.time() - wk.T0, 1)})
    wk.log(f"{status:4} send-to  {name:34} {dt:5.1f}s  {msg}")
    stash(folder, before)


# ------------------------------------------------------------------ main
EX = None
DESKTOP_SHOWN = [False]


def main():
    global EX
    wk.set_log(OUT / "log.txt")
    DESK_BEFORE.update(p.name for p in DESK.iterdir())
    wk.prepare_desktop(local=LOCAL)
    for f in DESK.glob("*"):
        if LOCAL:  # only this script's own leftovers
            if f.is_file() and f.name.startswith("D-") and f.name[2:] in SOURCES:
                f.unlink()
                DESK_BEFORE.discard(f.name)
        elif f.is_file() and f.suffix.lower() in (".jpg", ".png", ".pdf", ".docx", ".txt"):
            f.unlink()
    shutil.rmtree(EXP, ignore_errors=True)
    shutil.rmtree(STASH, ignore_errors=True)
    make_files(EXP)
    make_files(DESK, "D-")
    app = wk.start_app()
    time.sleep(5)
    EX = wk.open_explorer(EXP)
    rec = wk.start_recording(OUT / "e2e.mp4")
    time.sleep(1.5)
    wk.T0 = time.time()
    try:
        # ---- conversions from Explorer
        for fmt in ["PNG", "WEBP", "HEIC", "TIFF", "AVIF", "BMP", "PDF", "ICO", "GIF"]:
            ext = {"TIFF": ".tiff", "HEIC": ".heic"}.get(fmt, "." + fmt.lower())
            run(f"JPG → {fmt}", "explorer", ["Photo.jpg"], "Convert", fmt, v_format(fmt, ext))
        run("PNG → JPG", "explorer", ["Graphic.png"], "Convert", "JPG", v_format("JPEG", ".jpg"))
        run("PDF → JPG (3 pages)", "explorer", ["Report.pdf"], "Convert", "JPG", v_pages_to_images(3, ".jpg"))
        run("PDF → PNG (3 pages)", "explorer", ["Report.pdf"], "Convert", "PNG", v_pages_to_images(3, ".png"))
        run("PDF → DOCX", "explorer", ["Report.pdf"], "Convert", "DOCX", v_format("DOCX", ".docx"))
        run("PDF → TXT", "explorer", ["Report.pdf"], "Convert", "TXT", v_format("TXT", ".txt"))
        run("DOCX → PDF", "explorer", ["Letter.docx"], "Convert", "PDF", v_format("PDF", ".pdf"))
        run("DOCX → TXT", "explorer", ["Letter.docx"], "Convert", "TXT", v_format("TXT", ".txt"))
        run("TXT → PDF", "explorer", ["Notes.txt"], "Convert", "PDF", v_format("PDF", ".pdf"))
        run("TXT → DOCX", "explorer", ["Notes.txt"], "Convert", "DOCX", v_format("DOCX", ".docx"))
        run("2 images → 1 PDF", "explorer", ["Photo.jpg", "Second.jpg"], "Convert", "PDF", v_combined,
            select_extra=True)
        # ---- image tools
        run("Compress to 200 KB", "explorer", ["Photo.jpg"], "Tools", "Compress", v_compress(200_000), drive_compress)
        run("Resize 50%", "explorer", ["Photo.jpg"], "Tools", "Resize",
            v_dims(lambda a, b: b == (a[0] // 2, a[1] // 2)), drive_resize)
        run("Crop 1:1", "explorer", ["Photo.jpg"], "Tools", "Crop", v_dims(lambda a, b: b[0] == b[1] == min(a)), drive_crop)
        run("Edit: auto + rotate", "explorer", ["Photo.jpg"], "Tools", "Edit",
            v_dims(lambda a, b: b == (a[1], a[0])), drive_edit)
        run("Annotate", "explorer", ["Photo.jpg"], "Tools", "Annotate", v_exists, drive_annotate)
        run("Redact", "explorer", ["Photo.jpg"], "Tools", "Redact", v_redact, drive_redact)
        run("Add BG 1:1", "explorer", ["Photo.jpg"], "Tools", "Add BG", v_dims(lambda a, b: b[0] == b[1]), drive_addbg)
        run("Metadata: remove location", "explorer", ["Photo.jpg"], "Tools", "Metadata", v_no_gps_keep_camera,
            drive_meta_location)
        run("Metadata: remove all", "explorer", ["Photo.jpg"], "Tools", "Metadata", v_no_meta, drive_meta_all)
        # ---- PDF tools
        run("PDF Compress", "explorer", ["Report.pdf"], "Tools", "Compress", v_pdf_compress)
        run("PDF Split", "explorer", ["Report.pdf"], "Tools", "Split", v_pdf_n(3))
        run("PDF Rotate", "explorer", ["Report.pdf"], "Tools", "Rotate", v_rotated)
        run("PDF Protect", "explorer", ["Report.pdf"], "Tools", "Protect", v_protected, drive_protect)
        run("PDF Extract images", "explorer", ["Report.pdf"], "Tools", "Extract", v_extract)
        run("PDF Metadata", "explorer", ["Report.pdf"], "Tools", "Metadata", v_pdf_meta, drive_meta_all)
        run("Merge 2 PDFs", "explorer", ["Appendix.pdf", "Report.pdf"], "Tools", "Merge", v_merged,
            select_extra=True)
        # ---- the More and Pack rings
        run("More: long side 1280", "explorer", ["Photo.jpg"], "More", "1280 px", v_long_side(1280))
        run("More: no GPS", "explorer", ["Photo.jpg"], "More", "No GPS", v_no_gps_keep_camera)
        run("More: Copy image", "explorer", ["Second.jpg"], "More", "Copy", v_clipboard_image, expect_files=False)
        run("More: Text (OCR)", "explorer", ["Sign.png"], "More", "Text", v_ocr)
        run("More: PDF Gray", "explorer", ["Report.pdf"], "More", "Gray", v_gray_pdf)
        run("More: Stamp in Vietnamese", "explorer", ["Report.pdf"], "More", "Stamp", v_watermark_vi,
            drive_watermark_vi)
        run("Pack: ZIP", "explorer", ["Photo.jpg"], "Pack", "ZIP", v_zip)
        run("Pack: TAR.GZ", "explorer", ["Notes.txt"], "Pack", "TAR.GZ", v_tar)
        run("Extract archive, then Undo", "explorer", ["Bundle.zip"], "More", "Extract", v_extracted,
            after=undo_and_check)
        # ---- names and selections people really have
        run("Vietnamese name: JPG → PNG", "explorer", ["Ảnh chụp.jpg"], "Convert", "PNG", v_format("PNG", ".png"))
        run("3 files → one ZIP", "explorer", ["Notes.txt", "Letter.docx", "Graphic.png"], "Pack", "ZIP",
            v_zip_many(["Notes.txt", "Letter.docx", "Graphic.png"]), select_extra=True)
        run("Convert, then Undo", "explorer", ["Second.jpg"], "Convert", "WEBP", v_format("WEBP", ".webp"),
            after=undo_file_and_check)
        # ---- video and audio (system ffmpeg)
        run("Video → GIF", "explorer", ["Clip.mp4"], "Convert", "GIF", v_gif)
        run("Video → WEBM", "explorer", ["Clip.mp4"], "Convert", "WEBM", v_media(".webm", "Video"))
        run("Video → MP3", "explorer", ["Clip.mp4"], "Convert", "MP3", v_media(".mp3", "Audio"))
        run("Video Compress", "explorer", ["Clip.mp4"], "More", "Compress", v_media(".mp4", "Video"))
        # ---- second launch with a file (Send To / right-click path)
        run_ipc("Send To → PNG", "Photo.jpg", "PNG", v_format("PNG", ".png"))
        # ---- from the Desktop
        run("Desktop: JPG → PNG", "desktop", ["D-Photo.jpg"], "Convert", "PNG", v_format("PNG", ".png"))
        run("Desktop: PDF → TXT", "desktop", ["D-Report.pdf"], "Convert", "TXT", v_format("TXT", ".txt"))
        run("Desktop: DOCX → PDF", "desktop", ["D-Letter.docx"], "Convert", "PDF", v_format("PDF", ".pdf"))
        run("Desktop: Compress", "desktop", ["D-Photo.jpg"], "Tools", "Compress", v_compress(200_000), drive_compress)
        run("Desktop: Crop", "desktop", ["D-Photo.jpg"], "Tools", "Crop", v_dims(lambda a, b: b[0] == b[1]), drive_crop)
        run("Desktop: Add BG", "desktop", ["D-Graphic.png"], "Tools", "Add BG", v_dims(lambda a, b: b[0] == b[1]),
            drive_addbg)
        run("Desktop: 2 PDFs merge", "desktop", ["D-Appendix.pdf", "D-Report.pdf"], "Tools", "Merge", v_merged,
            select_extra=True)
        run("Desktop: Pack ZIP", "desktop", ["D-Photo.jpg"], "Pack", "ZIP", v_zip)
        run("Desktop: Extract, then Undo", "desktop", ["D-Bundle.zip"], "More", "Extract", v_extracted,
            after=undo_and_check)
        run("Desktop: Video → GIF", "desktop", ["D-Clip.mp4"], "Convert", "GIF", v_gif)
        # ---- every colour theme, on the real menu
        if DESKTOP_SHOWN[0]:
            EX.set_focus()
            DESKTOP_SHOWN[0] = False
            time.sleep(0.6)
        app = theme_tour(app)
    except Exception:
        wk.log("E2E ABORTED\n" + traceback.format_exc())
    finally:
        pyautogui.mouseUp()
        pyautogui.keyUp("shift")
        time.sleep(1)
        wk.stop_recording(rec)
        wk.stop_app(app)
        if LOCAL:  # leave someone's PC as it was: no sample files on the Desktop or in C:\
            wk.close_explorer(EXP)
            for n in SOURCES:
                p = DESK / f"D-{n}"
                if p.is_file() and p.name not in DESK_BEFORE:
                    p.unlink()
            shutil.rmtree(EXP, ignore_errors=True)
            shutil.rmtree(STASH, ignore_errors=True)
        wk.copy_app_log(OUT / "app_log.txt")
        try:  # the drag picture must never be baked into the glass
            log_text = (OUT / "app_log.txt").read_text(encoding="utf-8", errors="replace")
            n = log_text.count("drag image(s) during capture")
            RESULTS.append({"name": "Drag picture kept out of the glass", "where": "all",
                            "status": "PASS" if n > 0 else "FAIL",
                            "detail": f"hidden during {n} menu captures", "seconds": 0, "t": 0})
        except Exception as e:
            wk.log("drag image check failed", e)
        passed = sum(r["status"] == "PASS" for r in RESULTS)
        (OUT / "results.json").write_text(json.dumps(RESULTS, indent=1))
        lines = [f"# Utsuri end-to-end test — {passed}/{len(RESULTS)} passed", "",
                 "| # | Where | Scenario | Result | Detail | Time |", "|---|---|---|---|---|---|"]
        for i, r in enumerate(RESULTS, 1):
            mark = "✅" if r["status"] == "PASS" else "❌"
            lines.append(f"| {i} | {r['where']} | {r['name']} | {mark} | {r['detail']} | {r['seconds']}s |")
        (OUT / "results.md").write_text("\n".join(lines), encoding="utf-8")
        wk.log(f"DONE {passed}/{len(RESULTS)} passed")


if __name__ == "__main__":
    main()
