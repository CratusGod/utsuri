"""The desktop menu's extra rings (utsuri.more) and the jobs behind them."""
from __future__ import annotations

import shutil
import subprocess
import zipfile
from pathlib import Path

import pytest
from PIL import Image

from utsuri import app as A
from utsuri import more, report


def rings_for(paths) -> dict:
    """UtsuriApp.build_rings without starting the app (it never touches self's Qt state)."""
    class Bare:
        _upstream_rings = A.UtsuriApp._upstream_rings
    return A.UtsuriApp.build_rings(Bare(), [str(p) for p in paths])


def labels(rings: dict, name: str) -> list[str]:
    return [i.label for i in rings.get(name, [])]


def run(key: str, paths, copied=None) -> report.Result:
    copied = copied if copied is not None else []
    r = more.job(key, [str(p) for p in paths], copied.append, copied.append)()
    assert isinstance(r, report.Result), r
    return r


def test_added_slice_labels_fit_the_menu(png, pdf, tmp_path):
    for p in (png, pdf, tmp_path):
        for items in more.rings([str(p)]).values():
            assert all(len(i.label) <= 9 for i in items), [i.label for i in items]


def test_extra_slices_are_spread_evenly(pdf):
    sizes = [len(v) for k, v in more.rings([str(pdf)]).items() if k.startswith("More")]
    assert max(sizes) - min(sizes) <= 1, sizes


def test_no_added_ring_has_more_than_eight_slices(png, pdf, tmp_path):
    for p in (png, pdf, tmp_path):
        for name, items in more.rings([str(p)]).items():
            assert 0 < len(items) <= more.MAX_SLICES, (p, name, len(items))


def test_an_image_keeps_upstream_rings_and_gains_more(png):
    r = rings_for([png])
    assert "PNG" not in labels(r, "Convert") and "JPG" in labels(r, "Convert")
    assert "Crop" in labels(r, "Tools")
    assert {"≤ 500 KB", "No GPS", "Copy"} <= set(labels(r, "More"))
    assert "Compress" not in labels(r, "More"), "upstream's Tools ring has it already"
    assert "ZIP" in labels(r, "Pack")


def test_a_pdf_gains_the_page_windows_and_toolkit(pdf):
    r = rings_for([pdf])
    more_labels = labels(r, "More") + labels(r, "More 2")
    assert {"Organize", "Stamp", "Unlock", "Numbers", "Gray"} <= set(more_labels)


def test_a_folder_offers_batch_images_and_packing(tmp_path, png):
    folder = tmp_path / "shots"
    folder.mkdir()
    shutil.copy(png, folder / "a.png")
    r = rings_for([folder])
    assert "Unsupported" not in r
    assert {"→ WEBP", "Compress"} <= set(labels(r, "More"))
    assert "ZIP" in labels(r, "Pack")


def test_an_unknown_file_can_still_be_packed(tmp_path):
    f = tmp_path / "notes.xyz"
    f.write_bytes(b"\x00\x01")
    r = rings_for([f])
    assert list(r) == ["Pack"]


def test_pack_then_extract_round_trip(tmp_path, png):
    r = run("x:zip", [png])
    z = r.outs[0]
    assert z.suffix == ".zip" and zipfile.ZipFile(z).namelist() == [png.name]
    assert "Extract" in labels(rings_for([z]), "More")
    out = run("x:extract", [z]).outs[0]
    assert (out if out.is_file() else out / png.name).exists()


def test_page_numbers_write_a_new_pdf_and_keep_the_original(pdf):
    before = pdf.read_bytes()
    r = run("x:numbers", [pdf])
    assert r.outs[0].exists() and r.outs[0] != pdf
    assert pdf.read_bytes() == before


def test_copy_path_and_copy_image_go_through_the_given_clipboard(png):
    got = []
    run("x:copy-path", [png], got)
    assert got == [str(png)]
    got.clear()
    run("x:copy", [png], got)
    assert got == [png]


def test_a_refusal_is_raised_not_swallowed(pdf):
    with pytest.raises(more.cli.Refusal):
        more.job("x:no-such-tool", [str(pdf)], print, print)()


@pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="needs ffmpeg")
def test_a_video_gets_a_convert_ring_and_converts(tmp_path):
    v = tmp_path / "clip.mp4"
    subprocess.run(["ffmpeg", "-nostdin", "-loglevel", "error", "-f", "lavfi", "-i",
                    "testsrc=duration=1:size=64x48:rate=5", "-pix_fmt", "yuv420p", str(v)], check=True)
    r = rings_for([v])
    assert "WEBM" in labels(r, "Convert") and "Compress" in labels(r, "More")
    out = run("xc:WEBM", [v]).outs[0]
    assert out.suffix == ".webm" and out.stat().st_size > 0


def test_undo_removes_an_extracted_folder_but_no_other_folder(tmp_path, png):
    from PySide6.QtWidgets import QApplication
    from utsuri.toasts import trash
    QApplication.instance() or QApplication([])
    z2 = run("x:zip", [png]).outs[0]
    import zipfile as zf
    with zf.ZipFile(z2, "a") as f:
        f.writestr("second.txt", "x")  # two entries: extract makes a folder
    out = run("x:extract", [z2]).outs[0]
    assert out.is_dir()
    stranger = tmp_path / "keep me"
    stranger.mkdir()
    assert trash([out, stranger]) == 1
    assert not out.exists() and stranger.exists()
