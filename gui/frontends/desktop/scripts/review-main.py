#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import yaml


def execute(*arguments):
    return subprocess.check_output(arguments, text=True).strip()


def wait(predicate):
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        threading.Event().wait(0.1)
    raise RuntimeError("native View condition timed out")


def capture(binary, original, destination, settings_review=False):
    if settings_review:
        import atspi_layout as layout
    output = Path(destination)
    output.mkdir(parents=True, exist_ok=True)
    (output / "evidence.json").unlink(missing_ok=True)
    records = []
    with tempfile.TemporaryDirectory(prefix="nerust-view-rpa-") as temporary:
        root = Path(temporary)
        rom = root / Path(original).name
        shutil.copyfile(original, rom)
        settings = {"shared": {}, "app_state": {}, "local": {"audio": {"muted": True}, "notifications": {"status_enabled": True, "info_duration_ms": 10000}}}
        for directory in [root / "config" / "nerust", root / "settings" / "config"]:
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "settings.yaml").write_text(yaml.safe_dump(settings))
        environment = dict(os.environ, NO_AT_BRIDGE="0" if settings_review else "1", GTK_A11Y="atspi" if settings_review else os.environ.get("GTK_A11Y", "none"), GDK_BACKEND="x11",
                           XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
                           NERUST_SETTINGS_ROOT=str(root / "settings"))
        for mode, arguments in [("empty", []), ("rom", [str(rom)])]:
            with (output / f"{mode}.log").open("w") as log:
                process = subprocess.Popen([binary, *arguments], env=environment, stdout=log, stderr=log)
                try:
                    def find_window():
                        if process.poll() is not None:
                            raise RuntimeError(f"{mode}: application exited; inspect log")
                        result = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(process.pid)], capture_output=True, text=True)
                        return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                    window = wait(find_window)
                    execute("xdotool", "windowraise", window)
                    execute("xdotool", "windowfocus", "--sync", window)
                    execute("xdotool", "windowsize", "--sync", window, "660", "520")

                    def snapshot(name, expected=None, blank=False):
                        image = output / f"{name}.png"

                        def inspect():
                            execute("import", "-window", window, str(image))
                            text = execute("tesseract", str(image), "stdout", "--psm", "11")
                            menu_pixels = subprocess.check_output(["convert", str(image), "-crop", "400x24+0+0", "-resize", "300%", "png:-"])
                            menu_text = subprocess.check_output(["tesseract", "stdin", "stdout", "--psm", "7"], input=menu_pixels).decode()
                            if blank and "File" not in menu_text:
                                return None
                            width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                            status_top = 32 if "File" in menu_text else 8
                            status_pixels = subprocess.check_output(["convert", str(image), "-crop", f"{min(width - 8, 200)}x23+8+{status_top}",
                                "-resize", "400%", "-negate", "-threshold", "40%", "png:-"])
                            status = subprocess.check_output(["tesseract", "stdin", "stdout", "--psm", "7"], input=status_pixels).decode()
                            center = f"{width * 8 // 10}x{height * 6 // 10}+{width // 10}+{height // 5}"
                            colors = int(execute("convert", str(image), "-crop", center, "-format", "%k", "info:"))
                            darkness = float(execute("convert", str(image), "-crop", center, "-colorspace", "Gray", "-format", "%[fx:mean]", "info:"))
                            if (blank and colors > 8) or (not blank and colors <= 8):
                                return None
                            if expected and expected.casefold() not in (text + "\n" + status).casefold():
                                return None
                            return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
                                    "width": width, "height": height, "center_colors": colors, "center_mean": darkness,
                                    "ocr": text, "menu_ocr": menu_text, "status_ocr": status}

                        record = wait(inspect)
                        records.append(record)
                        print(f"VIEW CAPTURE {name}", flush=True)

                    if mode == "empty":
                        snapshot("empty", blank=True)
                    else:
                        snapshot("running")
                        execute("xdotool", "mousemove", "--window", window, "330", "260", "click", "1")
                        execute("xdotool", "key", "space")
                        snapshot("paused", expected="Paus")
                        if settings_review:
                            execute("xdotool", "key", "ctrl+comma")

                            def find_settings():
                                result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^nerust Settings$"], capture_output=True, text=True)
                                return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                            settings_window = wait(find_settings)
                            execute("xdotool", "windowfocus", "--sync", settings_window)
                            execute("xdotool", "windowsize", "--sync", settings_window, "900", "560")

                            def settings_frame():
                                return layout.window(process.pid, execute("xdotool", "getwindowname", settings_window))

                            def settings_snapshot(page, expected):
                                image = output / f"settings-{page.lower()}.png"
                                def inspect():
                                    if process.poll() is not None:
                                        raise RuntimeError("application exited during Settings capture")
                                    frame = settings_frame()
                                    if frame is None:
                                        return None
                                    execute("import", "-window", settings_window, str(image))
                                    text = execute("tesseract", str(image), "stdout", "--psm", "11")
                                    tree_file = output / f"settings-{page.lower()}.atspi.json"
                                    tree_file.write_text(json.dumps(layout.tree(frame), indent=2) + "\n")
                                    visible = [node for node in layout.descendants(frame) if layout.visible_in_frame(node, frame)]
                                    labels = " ".join(node.name for node in visible)
                                    if not all(layout.normalized(word) in layout.normalized(labels) for word in expected):
                                        return None
                                    if page == "Notifications":
                                        sections = [layout.bounds(node) for node in visible if node.name in {"Information", "Warning"}]
                                        if len(sections) != 2 or sections[0] == sections[1]:
                                            raise RuntimeError("Notifications sections overlap or are missing")
                                    width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                                    return {"state": f"settings-{page.lower()}", "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height, "ocr": text, "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest()}
                                records.append(wait(inspect))
                                print(f"VIEW CAPTURE settings-{page.lower()}", flush=True)

                            def click_page(page):
                                def locate():
                                    frame = settings_frame()
                                    return frame is not None and layout.select_page(frame, page)
                                wait(locate)

                            settings_snapshot("General", ["Language", "Storage", "Apply"])
                            for page, expected in [("Video", ["VSync", "Scaling", "Apply"]), ("Audio", ["Volume:", "Latency:", "Apply"]), ("Notifications", ["Information", "Warning", "Apply"])]:
                                click_page(page)
                                settings_snapshot(page, expected)
                            def scroll_notifications_to_end():
                                frame = settings_frame()
                                if frame is None:
                                    return False
                                for node in layout.descendants(frame):
                                    if node.getRoleName() == "scroll bar" and layout.visible_in_frame(node, frame):
                                        try:
                                            value = node.queryValue()
                                            if value.maximumValue > value.minimumValue:
                                                return value.setCurrentValue(value.maximumValue)
                                        except NotImplementedError:
                                            rectangle = layout.bounds(node)
                                            execute("xdotool", "mousemove", str(rectangle["x"] + rectangle["width"] // 2), str(rectangle["y"] + rectangle["height"] - 2), "click", "1")
                                            return True
                                return False
                            wait(scroll_notifications_to_end)
                            settings_snapshot("Notifications-bottom", ["Status display", "Overlay", "Apply"])
                            execute("xdotool", "windowfocus", "--sync", window)
                    execute("xdotool", "key", "alt+F4")
                finally:
                    if process.poll() is None:
                        process.terminate()
                    process.wait(timeout=15)
    (output / "evidence.json").write_text(json.dumps({"os": execute("uname", "-a"),
        "rom_sha256": hashlib.sha256(Path(original).read_bytes()).hexdigest(), "comparison_settings": settings,
        "captures": records}, indent=2) + "\n")
    print(f"VIEW PASS captures={len(records)}", flush=True)


if __name__ == "__main__":
    try:
        if len(sys.argv) not in (4, 5) or (len(sys.argv) == 5 and sys.argv[4] != "--settings"):
            raise RuntimeError("usage: review-main.py BINARY ROM OUTPUT [--settings]")
        capture(*sys.argv[1:4], settings_review=len(sys.argv) == 5)
    except Exception as error:
        print(f"VIEW FAIL: {error}", file=sys.stderr)
        sys.exit(1)