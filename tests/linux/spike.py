#!/usr/bin/env python3
"""Go/no-go: one real Shift+drag of a JPG from a test window onto Utsuri's menu, dropped on
the 12 o'clock slice (PNG). Prints what happened at each stage."""
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import desk  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
UTSURI = Path(os.environ.get("UTSURI_BIN", ROOT / "target/debug/utsuri"))
WORK = Path(os.environ.get("SPIKE_DIR", "/tmp/utsuri-spike"))


def main() -> int:
    shutil.rmtree(WORK, ignore_errors=True)
    (WORK / "files").mkdir(parents=True)
    subprocess.run([str(UTSURI), "--cli", "fixtures", str(WORK / "files")], check=True, stdout=subprocess.DEVNULL)
    photo = next(p for p in (WORK / "files").iterdir() if p.suffix == ".jpg")
    (WORK / "control").write_text(f"{photo}\n")
    log = WORK / "utsuri.log"
    env = dict(os.environ, UTSURI_SETTINGS=str(WORK / "settings.json"), UTSURI_STATE_DIR=str(WORK / "state"),
               UTSURI_FAKE_TRASH=str(WORK / "trash"), UTSURI_PIPE="utsuri-spike", UTSURI_LOG=str(log),
               UTSURI_NO_WELCOME="1", UTSURI_NO_LAUNCH="1")
    desk.ensure_ydotoold()
    home_ws = desk.current_workspace()
    desk.workspace("9")
    src = subprocess.Popen([sys.executable, str(Path(__file__).parent / "drag_source.py"), str(WORK / "control")])
    # the trigger needs the input group: newgrp gives this run the new group without a re-login
    app = subprocess.Popen(["newgrp", "input"], stdin=subprocess.PIPE, env=env, text=True)
    app.stdin.write(f"exec {UTSURI} --quiet\n")
    app.stdin.close()
    try:
        w = desk.wait_window(title="utsuri-e2e-source")
        desk.place(w["address"], 80, 120, 300, 300)
        time.sleep(1.5)  # Utsuri warms its menu at start
        desk.jump_to(230, 270)
        desk.button(True)
        desk.move_to(600, 400, steps=10)
        desk.key("shift", True)
        desk.move_to(604, 402, steps=3)  # the menu opens where the key went down
        time.sleep(0.6)
        desk.key("shift", False)
        desk.move_to(600, 400 - 110, steps=6)
        time.sleep(0.3)
        desk.button(False)
        time.sleep(3)
    finally:
        desk.release_all()
        app.terminate()
        src.terminate()
        desk.workspace(home_ws)
    print("--- log")
    print(log.read_text() if log.exists() else "(no log)")
    print("--- files")
    for p in sorted((WORK / "files").iterdir()):
        print(p.name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
