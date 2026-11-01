"""Translate with Oumu: text extraction, the request on Oumu's pipe, and the slice."""
from __future__ import annotations

import sys
import threading
import uuid
from pathlib import Path

import pytest

from conftest import eng
from utsuri import more, oumu_link


@pytest.fixture
def fake_oumu(monkeypatch):
    """A pipe server standing in for Oumu; yields the list of requests it got."""
    if sys.platform != "win32":
        pytest.skip("Oumu's pipe is Windows-only")
    from multiprocessing.connection import Listener
    name = f"\\\\.\\pipe\\oumu-test-{uuid.uuid4().hex[:8]}"
    monkeypatch.setenv("OUMU_PIPE", name)
    got: list[bytes] = []
    listener = Listener(address=name, family="AF_PIPE")

    def serve():
        try:
            with listener.accept() as conn:
                got.append(conn.recv_bytes())
        except Exception:  # noqa: BLE001 - the test failed elsewhere
            pass
    t = threading.Thread(target=serve, daemon=True)
    t.start()
    yield got
    listener.close()


def test_send_writes_the_translate_request(fake_oumu):
    assert oumu_link.running()
    oumu_link.send("Hôm nay trời đẹp")
    for _ in range(50):
        if fake_oumu:
            break
        threading.Event().wait(0.05)
    assert fake_oumu == ["translate\nHôm nay trời đẹp".encode()]


def test_no_oumu_says_so(monkeypatch):
    monkeypatch.setenv("OUMU_PIPE", "\\\\.\\pipe\\oumu-nobody-here")
    monkeypatch.setattr(oumu_link, "find_exe", lambda: None)
    with pytest.raises(oumu_link.OumuError, match="isn't running"):
        oumu_link.send("hello")


def test_empty_text_is_refused():
    with pytest.raises(oumu_link.OumuError, match="no text"):
        oumu_link.send("  \n ")


def test_text_of_text_word_and_pdf(tmp_path, pdf):
    t = tmp_path / "note.txt"
    t.write_text("Xin chào, thế giới", encoding="utf-8")
    assert oumu_link.text_of(t) == "Xin chào, thế giới"
    from docx import Document
    d = Document()
    d.add_paragraph("Bonjour")
    d.add_paragraph("le monde")
    d.save(tmp_path / "letter.docx")
    assert oumu_link.text_of(tmp_path / "letter.docx") == "Bonjour\nle monde"
    assert oumu_link.text_of(pdf).strip()


def test_images_go_through_ocr(png):
    assert oumu_link.text_of(png, ocr=lambda p: "read from " + p.name) == "read from " + png.name
    with pytest.raises(oumu_link.OumuError):
        oumu_link.text_of(png)


def test_long_text_is_cut_on_a_character_boundary():
    text, cut = oumu_link.clip("ả" * 40_000)  # 3 bytes each in UTF-8
    assert cut and len(text.encode("utf-8")) <= oumu_link.MAX_BYTES
    assert oumu_link.clip("short") == ("short", False)


@pytest.mark.parametrize("available", [True, False])
def test_the_slice_shows_only_when_oumu_is_there(tmp_path, monkeypatch, available):
    monkeypatch.setattr(oumu_link, "available", lambda: available)
    t = tmp_path / "note.txt"
    t.write_text("hello", encoding="utf-8")
    keys = [i.key for items in more.rings([str(t)]).values() for i in items]
    assert ("x:translate" in keys) is available


def test_the_slice_sends_every_file_text(tmp_path, monkeypatch):
    sent = []
    monkeypatch.setattr(oumu_link, "send", sent.append)
    a, b = tmp_path / "a.txt", tmp_path / "b.txt"
    a.write_text("first", encoding="utf-8")
    b.write_text("second", encoding="utf-8")
    res = more.job("x:translate", [str(a), str(b)], None, None)()
    assert sent == ["first\n\nsecond"] and res.title == "Sent to Oumu"


def test_the_slice_turns_oumu_errors_into_a_message(tmp_path, monkeypatch):
    def fail(text):
        raise oumu_link.OumuError("Oumu isn't running. Start Oumu and try again.")
    monkeypatch.setattr(oumu_link, "send", fail)
    t = tmp_path / "a.txt"
    t.write_text("x", encoding="utf-8")
    with pytest.raises(eng.Refusal, match="isn't running"):
        more.job("x:translate", [str(t)], None, None)()


def test_the_pipe_name_matches_oumus(monkeypatch):
    monkeypatch.delenv("OUMU_PIPE", raising=False)
    monkeypatch.setenv("USERNAME", "Hilda Lê")  # Oumu keeps ASCII letters, digits and -_. too
    assert oumu_link.pipe_name() == "\\\\.\\pipe\\oumu-HildaL"


def test_the_ring_order_puts_translate_last(tmp_path, monkeypatch, pdf):
    monkeypatch.setattr(oumu_link, "available", lambda: True)
    rings = more.rings([str(pdf)])
    last_more = [k for k in rings if k.startswith("More")][-1]
    assert rings[last_more][-1].label == "Translate"
    assert Path(pdf).exists()
