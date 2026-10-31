#!/usr/bin/env python3
"""Capture real GTK/Tao settings through XTest on an isolated X11 desktop.

Requires xdotool, ImageMagick and Tesseract, plus an otherwise unused DISPLAY. Native
geometry assertions are part of verify.sh; these images are visual evidence.
"""
import hashlib
import csv
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target/layout-review"
OUTPUT.mkdir(parents=True, exist_ok=True)
os.chdir(ROOT)
os.environ.update(GDK_BACKEND="x11", GSK_RENDERER="cairo", GTK_A11Y="none")


def run(*arguments):
    return subprocess.check_output(arguments, timeout=30)


def xdo(*arguments):
    return run("xdotool", *map(str, arguments)).decode().strip()


def tab_text(image, *options):
    pixels = run("convert", str(image), "-white-threshold", "40%", "png:-")
    return subprocess.check_output(["tesseract", "stdin", "stdout", "--psm", "11", *options], input=pixels, timeout=30).decode()


def capture(window, destination, expected=(), psm="6", pixel_check=lambda _: True):
    previous = None
    stable = 0
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        run("import", "-window", window, str(destination))
        pixels = run("convert", str(destination), "rgb:-")
        digest = hashlib.sha256(pixels).digest()
        stable = stable + 1 if digest == previous else 0
        if stable >= 2 and len(set(pixels)) > 8 and pixel_check(pixels):
            text = run("tesseract", str(destination), "stdout", "--psm", psm).decode()
            if "audio" in expected:
                text += tab_text(destination)
            normalized = re.sub(r"[^a-z0-9]", "", text.lower())
            if all(value in normalized for value in expected):
                return
        previous = digest
    raise RuntimeError(f"drawing did not stabilize: {destination}")


def scroll(window, direction):
    xdo("mousemove", "--window", window, 10, 90)
    xdo("click", "--repeat", 30, "--delay", 0, direction)


def edit(window, y, text):
    xdo("mousemove", "--window", window, 60, y, "click", 1)
    xdo("key", "ctrl+a")
    xdo("type", "--clearmodifiers", text)


def controls_review():
    output = Path(os.environ.get("WAKUI_CONTROLS_REVIEW_OUTPUT", ROOT / "target/controls-review"))
    output.mkdir(parents=True, exist_ok=True)
    records = []
    for backend in ("gtk", "tao"):
        subprocess.run(["cargo", "build", "--locked", "-p", "wakui3-settings", "--no-default-features", "--features", f"backend-{backend}", "--example", "controls"], check=True)
        with (output / f"controls-{backend}.log").open("w") as log:
            process = subprocess.Popen([str(ROOT / "target/debug/examples/controls")], env={**os.environ, "WAKUI_CONTROLS_REVIEW": "1"}, stdout=log, stderr=subprocess.STDOUT)
            try:
                window = xdo("search", "--sync", "--onlyvisible", "--name", "^NES Controls$").splitlines()[-1]
                xdo("windowfocus", "--sync", window)
                for width, height in ((640, 660), (960, 660), (440, 360)):
                    xdo("windowsize", "--sync", window, width, height)
                    if backend == "tao":
                        deadline = time.monotonic() + 10
                        while f"CONTROLS3 DRAW {width}x{height}" not in (output / f"controls-{backend}.log").read_text():
                            if process.poll() is not None or time.monotonic() >= deadline:
                                raise RuntimeError(f"{backend}: resized frame was not drawn")
                    for state, direction, expected in (("top", 4, ("nearest",)), ("bottom", 5, ("audio", "settings"))):
                        scroll(window, direction)
                        image = output / f"controls-{backend}-{width}x{height}-{state}.png"
                        capture(window, image, expected, pixel_check=lambda pixels: any(pixels[-3:]))
                        records.append({"backend": backend, "state": state, "width": width, "height": height, "image": image.name})
                image = output / f"controls-{backend}-440x360-bottom.png"
                rows = csv.DictReader(io.StringIO(tab_text(image, "tsv")), delimiter="\t")
                tab_buttons = [row for row in rows if row["text"].lower() == "audio"]
                if not tab_buttons:
                    raise RuntimeError(f"{backend}: vertical Audio tab not found")
                tab_button = tab_buttons[-1]
                xdo("mousemove", "--window", window, int(tab_button["left"]) + int(tab_button["width"]) // 2, int(tab_button["top"]) + int(tab_button["height"]) // 2, "click", 1)
                capture(window, output / f"controls-{backend}-vertical-audio.png", ("audiosettings",))
                print(f"TABS3 CAPTURE {backend} vertical physical-click audio", flush=True)
                rows = csv.DictReader(io.StringIO(run("tesseract", str(image), "stdout", "--psm", "6", "tsv").decode()), delimiter="\t")
                buttons = [row for row in rows if row["text"].lower() == "settings"]
                if not buttons:
                    raise RuntimeError(f"{backend}: Settings button not found")
                button = buttons[-1]
                xdo("mousemove", "--window", window, int(button["left"]) + int(button["width"]) // 2, int(button["top"]) + int(button["height"]) // 2, "click", 1)
                dialog = xdo("search", "--sync", "--onlyvisible", "--name", "^Settings$").splitlines()[-1] if backend == "gtk" else window
                if backend == "gtk":
                    capture(dialog, output / f"controls-{backend}-dialog.png", ("done",), psm="11")
                else:
                    capture(dialog, output / f"controls-{backend}-dialog.png", ("settings",), psm="11", pixel_check=lambda pixels: all(100 < value < 230 for value in pixels[:3]))
                if process.poll() is not None:
                    raise RuntimeError(f"{backend}: controls closed unexpectedly")
                print(f"CONTROLS3 CAPTURE {backend} top bottom modal", flush=True)
            finally:
                process.terminate()
                process.wait(timeout=10)
    (output / "controls-captures.json").write_text(json.dumps(records, indent=2) + "\n")
    print(f"CONTROLS3 CAPTURE PASS {len(records)} states/sizes plus two modals; inspect {output}")


if "--controls" in sys.argv:
    controls_review()
    sys.exit(0)


records = []
for backend in ("gtk", "tao"):
    subprocess.run(
        ["cargo", "build", "--locked", "-p", "wakui3-settings", "--no-default-features", "--features", f"backend-{backend}"],
        check=True,
    )
    with tempfile.TemporaryDirectory(prefix="wakui3-layout-") as temporary:
        with (OUTPUT / f"{backend}.log").open("w") as log:
            process = subprocess.Popen(
                [str(ROOT / "target/debug/wakui3-settings"), "--config", str(Path(temporary) / "settings.json")],
                stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                windows = xdo("search", "--sync", "--onlyvisible", "--name", "Wakui 3.*Settings").splitlines()
                window = windows[-1]
                xdo("windowfocus", "--sync", window)
                for state in ("initial", "invalid", "confirm"):
                    xdo("windowsize", "--sync", window, 660, 520)
                    scroll(window, 4)
                    capture(window, OUTPUT / f"{backend}-ready.png")
                    if state == "invalid":
                        edit(window, 185, "abc")
                    elif state == "confirm":
                        edit(window, 185, "42")
                        edit(window, 113, "LayoutPlayer")
                        capture(window, OUTPUT / f"{backend}-dirty-ready.png", ("layoutplayer", "unsavedchanges"))
                        xdo("key", "ctrl+q")
                    for width, height in ((660, 520), (960, 520), (440, 360)):
                        xdo("windowsize", "--sync", window, width, height)
                        scroll(window, 4)
                        destination = OUTPUT / f"{backend}-{state}-{width}x{height}-top.png"
                        expected = ("playerpreferences",)
                        if height == 520:
                            expected += {"initial": ("uptodate",), "invalid": ("volumemustbeaninteger",), "confirm": ("discardorkeepediting",)}[state]
                        capture(window, destination, expected)
                        if process.poll() is not None:
                            raise RuntimeError(f"{backend} unexpectedly closed during {state}")
                        records.append({"backend": backend, "state": state, "width": width, "height": height, "image": destination.name})
                        if height == 360:
                            scroll(window, 5)
                            expected = {"initial": ("uptodate",), "invalid": ("unsavedchanges",), "confirm": ("keepediting", "unsavedchanges")}[state]
                            capture(window, OUTPUT / f"{backend}-{state}-{width}x{height}-bottom.png", expected)
                    print(f"LAYOUT3 CAPTURE {backend} {state}", flush=True)
            finally:
                process.terminate()
                process.wait(timeout=10)

(OUTPUT / "captures.json").write_text(json.dumps(records, indent=2) + "\n")
print(f"LAYOUT3 CAPTURE PASS {len(records)} states/sizes; inspect top/bottom images in {OUTPUT}")