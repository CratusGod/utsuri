"""Translate with Oumu: hand a file's text to Oumu, which opens its Translate panel.

Oumu (CratusGod/oumu) listens on a named pipe for `translate\\n<text>` (Windows),
the same request DragShift sends it on Linux. Text files, Word documents and PDFs
give their text; images go through OCR first. When Oumu is installed but not
running it is started and the text is sent once it listens.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

IS_WIN = sys.platform == "win32"
#: Oumu reads at most 64 KiB per request; leave room for the command line.
MAX_BYTES = 60_000
START_WAIT = 10.0


class OumuError(Exception):
    """A message that is fine to show the user as-is."""


def pipe_name() -> str:
    """Oumu's pipe for this user (win/remote.rs: the same name rules)."""
    if os.environ.get("OUMU_PIPE", "").startswith("\\\\.\\pipe\\"):
        return os.environ["OUMU_PIPE"]
    user = "".join(c for c in os.environ.get("USERNAME", "") if c.isascii() and (c.isalnum() or c in "-_."))
    return f"\\\\.\\pipe\\oumu-{user or 'user'}"


def running() -> bool:
    """Oumu's pipe exists. Listed, not opened: opening it (as os.path.exists does)
    would connect, and use up the connection Oumu is waiting on."""
    if not IS_WIN:
        return False
    name = pipe_name()[len("\\\\.\\pipe\\"):]
    try:
        return name in os.listdir("\\\\.\\pipe\\")
    except OSError:
        return False


def find_exe() -> str | None:
    """Oumu's executable: on PATH, or where its installers put it."""
    if not IS_WIN:
        return None
    found = shutil.which("oumu")
    if found:
        return found
    local, pf = os.environ.get("LOCALAPPDATA", ""), os.environ.get("ProgramFiles", "")
    for p in (Path(local) / "Oumu" / "oumu.exe", Path(local) / "Programs" / "Oumu" / "oumu.exe",
              Path(pf) / "Oumu" / "oumu.exe"):
        if p.is_file():
            return str(p)
    return None


def available() -> bool:
    """Show the Translate slice: Oumu runs, or can be started."""
    return running() or find_exe() is not None


def text_of(path: Path, ocr=None) -> str:
    """The text in a text file, Word document or PDF; an image through `ocr(path)`."""
    from .util import category
    cat = category(path)
    if cat == "text":
        return path.read_text(encoding="utf-8", errors="replace")
    if cat == "docx":
        from docx import Document
        return "\n".join(p.text for p in Document(str(path)).paragraphs)
    if cat == "pdf":
        import pymupdf
        with pymupdf.open(path) as d:
            return "\n".join(page.get_text("text") for page in d)
    if cat == "image" and ocr is not None:
        return ocr(path)
    raise OumuError(f"Utsuri can't read text from {path.name}")


def clip(text: str, limit: int = MAX_BYTES) -> tuple[str, bool]:
    """`text` cut to `limit` UTF-8 bytes on a character boundary; (text, was it cut)."""
    data = text.encode("utf-8")
    if len(data) <= limit:
        return text, False
    return data[:limit].decode("utf-8", errors="ignore"), True


def _send(text: str) -> bool:
    # Open-existing and write-only: open(..., "wb") would ask to create/truncate,
    # and a pipe Oumu only reads from refuses read access.
    try:
        fd = os.open(pipe_name(), os.O_WRONLY | getattr(os, "O_BINARY", 0))
    except OSError:
        return False
    try:
        os.write(fd, b"translate\n" + text.encode("utf-8"))
        return True
    except OSError:
        return False
    finally:
        os.close(fd)


def send(text: str) -> None:
    """Hand `text` to Oumu's Translate view, starting Oumu first when it isn't running."""
    if not text.strip():
        raise OumuError("There is no text in it to translate")
    if _send(text):
        return
    exe = find_exe()
    if exe is None:
        raise OumuError("Oumu isn't running. Start Oumu and try again.")
    flags = getattr(subprocess, "DETACHED_PROCESS", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
    subprocess.Popen([exe], creationflags=flags, close_fds=True,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    end = time.time() + START_WAIT
    while time.time() < end:
        time.sleep(0.3)
        if _send(text):
            return
    raise OumuError("Oumu did not start in time")
