"""Clipboard and OCR for Utsuri (wl-copy, tesseract).

wl-copy forks a helper that keeps serving the clipboard after we exit, so its
output is sent to /dev/null: a captured pipe would keep the job waiting until
the clipboard changes.
"""
from __future__ import annotations

import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

LANG_RE = re.compile(r"^[A-Za-z_]+(\+[A-Za-z_]+)*$")
IS_WIN = sys.platform == "win32"
TESSDATA = "/usr/share/tessdata"


class ExtrasError(Exception):
    """A message that is fine to show the user as-is."""


def _need(cmd: str, package: str) -> None:
    if shutil.which(cmd) is None:
        raise ExtrasError(f"{cmd} isn't installed. Install the '{package}' package.")


def tesseract() -> str | None:
    """The tesseract executable. On Windows the installers put it in Program Files
    without touching PATH, so look there too."""
    found = shutil.which("tesseract")
    if found or not IS_WIN:
        return found
    for base in (os.environ.get("ProgramFiles"), os.environ.get("ProgramFiles(x86)"),
                 os.path.join(os.environ.get("LOCALAPPDATA", ""), "Programs")):
        if base and os.path.isfile(exe := os.path.join(base, "Tesseract-OCR", "tesseract.exe")):
            return exe
    return None


def tessdata() -> str:
    """Where tesseract's language files are (MuPDF's searchable-PDF OCR needs it)."""
    if os.environ.get("TESSDATA_PREFIX"):
        return os.environ["TESSDATA_PREFIX"]
    exe = tesseract()
    if exe and os.path.isdir(d := os.path.join(os.path.dirname(exe), "tessdata")):
        return d
    return TESSDATA


def _need_tesseract() -> str:
    exe = tesseract()
    if exe is None:
        if IS_WIN:
            raise ExtrasError("Tesseract isn't installed. Run: winget install UB-Mannheim.TesseractOCR")
        raise ExtrasError("tesseract isn't installed. Install the 'tesseract' package.")
    return exe


def missing_lang_message(code: str) -> str:
    if IS_WIN:
        return (f"OCR language '{code}' isn't installed. Rerun the Tesseract installer "
                f"and tick it under Additional language data.")
    return f"OCR language '{code}' isn't installed. Install the 'tesseract-data-{code}' package."


def _wl_copy(data: bytes, mime: str) -> None:
    _need("wl-copy", "wl-clipboard")
    r = subprocess.run(["wl-copy", "--type", mime], input=data,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
    if r.returncode != 0:
        raise ExtrasError("wl-copy couldn't reach the clipboard (is this a Wayland session?)")


def copy_image(src: Path, open_image) -> None:
    """Put the image on the clipboard as PNG, which every app can paste."""
    if src.suffix.lower() == ".png":
        data = src.read_bytes()
    else:
        buf = io.BytesIO()
        with open_image(src) as im:
            im.save(buf, "PNG")
        data = buf.getvalue()
    _wl_copy(data, "image/png")


def copy_paths(paths: list[Path]) -> None:
    _wl_copy("\n".join(str(p) for p in paths).encode(), "text/plain;charset=utf-8")


_NO_WINDOW = subprocess.CREATE_NO_WINDOW if IS_WIN else 0  # no console flashing up from the GUI


def ocr_langs() -> list[str]:
    exe = tesseract()
    if exe is None:
        return []
    try:
        r = subprocess.run([exe, "--list-langs"], capture_output=True, text=True, timeout=15,
                           creationflags=_NO_WINDOW)
    except (OSError, subprocess.SubprocessError):
        return []
    return [ln.strip() for ln in r.stdout.splitlines()[1:] if ln.strip() and ln.strip() != "osd"]


def ocr(src: Path, lang: str, open_image, sibling, copy: bool = True) -> tuple[Path, str]:
    """Read the text in an image, save it as "<name> Text.txt" and (with copy) put it
    on the clipboard through wl-copy. The desktop app copies through Qt instead."""
    if not LANG_RE.match(lang):   # before anything else: the codes reach a command line
        raise ExtrasError(f"'{lang}' isn't a valid OCR language (use codes like eng or eng+vie)")
    exe = _need_tesseract()
    have = ocr_langs()
    missing = [code for code in lang.split("+") if code not in have]
    if missing:
        raise ExtrasError(missing_lang_message(missing[0]))
    with tempfile.TemporaryDirectory(prefix="utsuri-ocr-") as tmp:
        png = Path(tmp) / "page.png"
        with open_image(src) as im:  # HEIC/AVIF/WEBP become something leptonica reads
            im.save(png, "PNG")
        r = subprocess.run([exe, str(png), "stdout", "-l", lang], stdin=subprocess.DEVNULL,
                           capture_output=True, encoding="utf-8", errors="replace", timeout=300,
                           creationflags=_NO_WINDOW)
    text = r.stdout.strip()
    if r.returncode != 0:
        raise ExtrasError(f"tesseract failed: {(r.stderr.strip().splitlines() or ['no output'])[-1]}")
    if not text:
        raise ExtrasError("No text found in this image")
    out = sibling(src, "Text", ".txt")
    out.write_text(text + "\n", encoding="utf-8")
    if copy:
        _wl_copy(text.encode(), "text/plain;charset=utf-8")
    return out, text
