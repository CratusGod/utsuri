"""The menu slices beyond upstream's Convert and Tools rings: extra image and PDF
tools, video and audio, archives, folders, and packing anything into an archive.

The work itself is utsuri.cli's action tables, the same ones the Ryoku bar plugin
drives, so both front ends offer the same things. This module only lays them out
as rings of at most eight slices and turns a slice into a background job.
"""
from __future__ import annotations

from pathlib import Path

from . import archive, cli, extras, media, oumu_link, report
from .radial import Item

MAX_SLICES = 8
# Key prefixes: "x:<id>" runs cli's tool <id>, "xc:<target>" converts video/audio,
# "xw:<id>" opens one of utsuri.pdfwin's windows (Qt, so app.py builds those).
PDF_WINDOWS = {"organize": ("Organize", "Reorder · rotate · delete"),
               "watermark": ("Stamp", "Text or image watermark"),
               "unlock": ("Unlock", "Remove a password")}
# The radial menu fits about 8 characters per slice; cli's labels are for the bar panel.
SHORT = {"compress-500kb": "≤ 500 KB", "long-1920": "1920 px", "long-1280": "1280 px",
         "strip-location": "No GPS", "numbers": "Numbers", "gray": "Gray", "halves": "Halves",
         "chapters": "Chapters", "searchable": "OCR", "ocr": "Text", "batch-webp": "→ WEBP",
         "batch-jpg": "→ JPG", "batch-compress": "Compress", "copy-path": "Path"}
HINTS = {"compress-500kb": "For upload limits", "long-1920": "Long side 1920 px", "long-1280": "Long side 1280 px",
         "strip-location": "Keep the rest", "copy": "Image to clipboard", "ocr": "Copy the text in it",
         "numbers": "1, 2, 3 at the foot", "gray": "Black and white", "flatten": "Bake in forms",
         "halves": "Two files", "chapters": "One file per chapter", "searchable": "OCR every page",
         "extract": "Unpack beside it", "batch-webp": "Every image inside", "batch-jpg": "Every image inside",
         "batch-compress": "Every image inside", "copy-path": "To clipboard"}
# Tools already in upstream's rings: not offered twice.
UPSTREAM = {"compress", "half", "split", "rotate", "extract", "strip-all"}


def kind(paths: list[str]) -> tuple[str, list[Path], list[Path]]:
    """(kind, files of that kind, everything dropped) - cli's rules."""
    items = [Path(p) for p in paths if Path(p).is_file() or Path(p).is_dir()]
    if not items:
        return "any", [], []
    cat, files, _ = cli.same_kind((items, []))
    return cat, files, items


def _ocr_ready() -> bool:
    return "eng" in extras.ocr_langs()


def _slice(tid: str, label: str) -> Item:
    return Item(SHORT.get(tid, label), key=f"x:{tid}", hint=HINTS.get(tid, ""))


def rings(paths: list[str]) -> dict[str, list[Item]]:
    """Extra rings for this drop; app.py appends them after its own."""
    cat, files, everything = kind(paths)
    out: dict[str, list[Item]] = {}
    if cat in ("video", "audio") and media.available():
        out["Convert"] = [Item(t, key=f"xc:{t}", hint=media.HINTS.get(t, ""))
                          for t in media.targets_for(cat, files[0].suffix)]
    more = [_slice(k, v[0]) for k, v in cli.tools_for(cat).items() if k not in UPSTREAM]
    more += [_slice(k, v[0]) for k, v in cli.job_tools_for(cat).items()
             if (k not in ("ocr", "searchable") or _ocr_ready()) and not (cat == "archive" and k == "copy")]
    if cat == "video" and media.available():
        more.insert(0, _slice("compress", "Compress"))
    if cat == "pdf":
        more = [Item(v[0], key=f"xw:{k}", hint=v[1]) for k, v in PDF_WINDOWS.items()] + more
    if (cat in ("text", "docx", "pdf") or (cat == "image" and _ocr_ready())) and oumu_link.available():
        more.append(Item("Translate", key="x:translate", hint="Opens in Oumu"))
    # Spread evenly over as few rings as fit: 9 slices -> 5 + 4, never 8 + 1.
    n_rings = -(-len(more) // MAX_SLICES)
    per = -(-len(more) // n_rings) if n_rings else 0
    for k in range(n_rings):
        out["More" if k == 0 else f"More {k + 1}"] = more[k * per:(k + 1) * per]
    out["Pack"] = [Item(SHORT.get(k, v[0]), key=f"x:{k}",
                        hint=archive.FORMATS[k][3] if k in archive.FORMATS else HINTS.get(k, ""))
                   for k, v in cli.any_tools().items()]
    return out


def _result(r: dict, srcs: list[Path]) -> report.Result:
    if not r.get("ok"):
        raise cli.Refusal(r.get("error") or "That didn't work")
    return report.Result(r["title"], r["detail"], [Path(o) for o in r["outs"]], srcs, r.get("good", ""))


def job(key: str, paths: list[str], copy_text, copy_image):
    """A function for app.submit that runs this slice and returns a report.Result.
    copy_text / copy_image put things on the clipboard through Qt, which works on
    every platform (cli's own copy uses wl-copy, for the Ryoku plugin)."""
    cat, files, everything = kind(paths)
    tag, _, tid = key.partition(":")
    if tag == "xc":
        return lambda: _result(cli.cmd_convert(["--to", tid, *map(str, files)]), files)
    if tid == "copy-path":
        def run_copy_paths():
            copy_text("\n".join(str(p) for p in everything))
            n = len(everything)
            return report.Result("Path copied" if n == 1 else f"{n} paths copied", everything[0].name if n == 1 else "",
                                 [], everything)
        return run_copy_paths
    if tid == "copy":
        def run_copy_image():
            copy_image(files[0])
            return report.Result("Image copied", files[0].name, [], files)
        return run_copy_image
    if tid == "translate":
        def run_translate():
            ocr = lambda p: extras.ocr_text(p, cli.OPTIONS["ocr_lang"], cli.convert.open_image)  # noqa: E731
            try:
                text = "\n\n".join(oumu_link.text_of(f, ocr) for f in files)
                text, cut = oumu_link.clip(text)
                oumu_link.send(text)
            except oumu_link.OumuError as e:
                raise cli.Refusal(str(e)) from None
            name = files[0].name if len(files) == 1 else f"{len(files)} files"
            return report.Result("Sent to Oumu", f"{name}{' · the first part' if cut else ''} · translating", [], files)
        return run_translate
    if tid == "ocr":
        def run_ocr():
            out, text = extras.ocr(files[0], cli.OPTIONS["ocr_lang"], cli.convert.open_image, cli.util.sibling,
                                   copy=False)
            copy_text(text)
            words = len(text.split())
            return report.Result("Text copied", f"{out.name} · {words} word{'s' if words != 1 else ''}", [out], files)
        return run_ocr
    return lambda: _result(cli.cmd_tool([tid, *map(str, everything if tid in cli.any_tools() else files)]), files)
