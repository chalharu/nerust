#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

import pyatspi


def execute(*arguments):
    return subprocess.check_output(arguments, text=True).strip()


def wait(predicate, description="condition"):
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        threading.Event().wait(0.1)
    raise RuntimeError(f"master auxiliary View timed out: {description}")


def review(binary, original, destination):
    output = Path(destination)
    output.mkdir(parents=True, exist_ok=True)
    report = output / "evidence.json"
    report.unlink(missing_ok=True)
    records = []
    with tempfile.TemporaryDirectory(prefix="nerust-master-auxiliary-") as temporary:
        root = Path(temporary)
        rom = root / Path(original).name
        shutil.copyfile(original, rom)
        fixture = {"shared": {}, "app_state": {}, "local": {"audio": {"muted": True}, "notifications": {"status_enabled": True, "info_duration_ms": 10000}}}
        for folder in [root / "config" / "nerust", root / "settings" / "config"]:
            folder.mkdir(parents=True, exist_ok=True)
            (folder / "settings.yaml").write_text(json.dumps(fixture))
        environment = dict(os.environ, NO_AT_BRIDGE="0", GTK_A11Y="atspi", GDK_BACKEND="x11", XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"), NERUST_SETTINGS_ROOT=str(root / "settings"))
        with (output / "application.log").open("w") as log:
            process = subprocess.Popen([binary, str(rom)], env=environment, stdout=log, stderr=log)
            try:
                def find_window(pattern):
                    if process.poll() is not None:
                        raise RuntimeError("master exited; inspect application.log")
                    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", pattern], capture_output=True, text=True)
                    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                def focus(window):
                    execute("xdotool", "windowraise", window)
                    execute("xdotool", "windowfocus", "--sync", window)

                def normalized(text):
                    return re.sub(r"[^a-z0-9]", "", text.casefold())

                def descendants(accessible):
                    yield accessible
                    for index in range(accessible.childCount):
                        child = accessible.getChildAtIndex(index)
                        if child is not None:
                            yield from descendants(child)

                def application():
                    desktop = pyatspi.Registry.getDesktop(0)
                    for index in range(desktop.childCount):
                        candidate = desktop.getChildAtIndex(index)
                        if candidate.get_process_id() == process.pid:
                            return candidate
                    return None

                def accessible_window(window):
                    app = application()
                    if app is None:
                        return None
                    title = normalized(execute("xdotool", "getwindowname", window))
                    for accessible in descendants(app):
                        if accessible.getRoleName() in {"frame", "dialog", "window"} and normalized(accessible.name) == title:
                            return accessible
                    return None

                def bounds(accessible):
                    try:
                        rectangle = accessible.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                        return {"x": rectangle.x, "y": rectangle.y, "width": rectangle.width, "height": rectangle.height}
                    except (NotImplementedError, RuntimeError):
                        return None

                def tree(accessible, parent_bounds=None):
                    node = {"name": accessible.name, "role": accessible.getRoleName(), "showing": accessible.getState().contains(pyatspi.STATE_SHOWING), "bounds": bounds(accessible)}
                    node["states"] = [str(state) for state in accessible.getState().getStates()]
                    rectangle = node["bounds"]
                    if rectangle is not None and parent_bounds is not None:
                        node["parent_allocation_insets"] = {"left": rectangle["x"] - parent_bounds["x"], "top": rectangle["y"] - parent_bounds["y"], "right": parent_bounds["x"] + parent_bounds["width"] - rectangle["x"] - rectangle["width"], "bottom": parent_bounds["y"] + parent_bounds["height"] - rectangle["y"] - rectangle["height"]}
                    try:
                        action = accessible.queryAction()
                        node["actions"] = [action.getName(index) for index in range(action.nActions)]
                    except (NotImplementedError, RuntimeError):
                        node["actions"] = []
                    try:
                        text = accessible.queryText()
                        node["text"] = text.getText(0, text.characterCount)
                    except (NotImplementedError, RuntimeError):
                        pass
                    node["children"] = [tree(accessible.getChildAtIndex(index), rectangle) for index in range(accessible.childCount) if accessible.getChildAtIndex(index) is not None]
                    for previous, current in zip(node["children"], node["children"][1:]):
                        before, after = previous["bounds"], current["bounds"]
                        if before is not None and after is not None:
                            current["previous_sibling_distance"] = {"horizontal": after["x"] - before["x"] - before["width"], "vertical": after["y"] - before["y"] - before["height"]}
                    return node

                def in_frame(accessible, frame):
                    rectangle, origin = bounds(accessible), bounds(frame)
                    return rectangle is not None and origin is not None and rectangle["width"] > 0 and rectangle["height"] > 0 and rectangle["x"] < origin["x"] + origin["width"] and rectangle["y"] < origin["y"] + origin["height"] and rectangle["x"] + rectangle["width"] > origin["x"] and rectangle["y"] + rectangle["height"] > origin["y"]

                def snapshot(window, name, expected):
                    image = output / f"{name}.png"
                    def inspect():
                        frame = accessible_window(window)
                        if frame is None:
                            return None
                        labels = " ".join(accessible.name for accessible in descendants(frame) if in_frame(accessible, frame))
                        if not all(normalized(word) in normalized(labels) for word in expected):
                            return None
                        layout = tree(frame)
                        execute("import", "-window", window, str(image))
                        text = execute("tesseract", str(image), "stdout", "--psm", "11")
                        layout_file = output / f"{name}.atspi.json"
                        layout_file.write_text(json.dumps(layout, indent=2) + "\n")
                        width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                        return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height, "title": execute("xdotool", "getwindowname", window), "ocr": text, "layout": layout_file.name, "layout_sha256": hashlib.sha256(layout_file.read_bytes()).hexdigest()}
                    record = wait(inspect, f"capture {name}: {expected}")
                    records.append(record)
                    print(f"MASTER GTK CAPTURE {name}", flush=True)

                def click_text(window, text, sidebar=False):
                    def locate():
                        frame = accessible_window(window)
                        if frame is None:
                            return None
                        origin = bounds(frame)
                        for accessible in descendants(frame):
                            if normalized(accessible.name) != normalized(text) or not in_frame(accessible, frame):
                                continue
                            rectangle = bounds(accessible)
                            if rectangle is None or rectangle["width"] <= 0 or rectangle["height"] <= 0:
                                continue
                            if sidebar and rectangle["x"] - origin["x"] >= 200:
                                continue
                            target = accessible
                            for _ in range(3):
                                try:
                                    action = target.queryAction()
                                    for index in range(action.nActions):
                                        if action.getName(index) in {"click", "press", "activate", "button.activate", "row.activate"}:
                                            return lambda action=action, index=index: action.doAction(index), target.name, action.getName(index)
                                except (NotImplementedError, RuntimeError):
                                    pass
                                if sidebar and target.parent is not None:
                                    try:
                                        selection = target.parent.querySelection()
                                        index = target.getIndexInParent()
                                        return lambda selection=selection, index=index: selection.selectChild(index), target.name, "selectChild"
                                    except (NotImplementedError, RuntimeError):
                                        pass
                                target = target.parent
                                if target is None or target == frame:
                                    break
                        return None
                    perform, target_name, action_name = wait(locate, f"AT-SPI Action {text} in window {window}")
                    print(f"MASTER GTK ACTION {text}: {target_name} / {action_name}", flush=True)
                    if not perform():
                        raise RuntimeError(f"AT-SPI Action rejected: {text}")

                def window_action(window, name):
                    frame = wait(lambda: accessible_window(window), f"accessible window for {name}")
                    action = frame.queryAction()
                    for index in range(action.nActions):
                        if action.getName(index) == name:
                            print(f"MASTER GTK WINDOW ACTION {name}", flush=True)
                            if not action.doAction(index):
                                raise RuntimeError(f"AT-SPI window Action rejected: {name}")
                            return
                    raise RuntimeError(f"AT-SPI window Action missing: {name}")

                main = wait(lambda: find_window("."), "main native window owned by process")
                focus(main)
                execute("xdotool", "windowsize", "--sync", main, "660", "520")
                window_action(main, "win.settings")
                settings = wait(lambda: find_window("Preferences|Settings"), "Settings native window after Action")
                focus(settings)
                snapshot(settings, "settings-general", ["Language", "Storage", "Cancel"])
                for page, expected in [("Input", ["Input", "Cancel"]), ("Video", ["Scaling", "Cancel"]), ("Audio", ["Latency", "Cancel"]), ("Notifications", ["Warning", "Cancel"]), ("System", ["System", "Cancel"])]:
                    click_text(settings, page, sidebar=True)
                    snapshot(settings, f"settings-{page.lower()}", expected)
                execute("xdotool", "key", "Escape")
                focus(main)
                window_action(main, "win.debugger")
                debugger = wait(lambda: find_window("Debugger"))
                focus(debugger)
                snapshot(debugger, "debugger", ["Step", "PPU"])
                click_text(debugger, "PPU Viewer")
                ppu = wait(lambda: find_window("PPU"))
                focus(ppu)
                snapshot(ppu, "ppu", ["Pattern"])
                focus(main)
                window_action(main, "win.history")
                history = wait(lambda: find_window("Notification [Hh]istory"))
                focus(history)
                snapshot(history, "history", ["Level", "Clear", "Close"])
                report.write_text(json.dumps({"os": execute("uname", "-a"), "source_revision": os.environ.get("NERUST_SOURCE_REVISION"), "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(), "display_geometry": execute("xdotool", "getdisplaygeometry"), "rom_sha256": hashlib.sha256(Path(original).read_bytes()).hexdigest(), "requested_settings_fixture": fixture, "measurement_units": "screen pixels", "insets_semantics": "Distances between accessible parent and child allocations, not inferred CSS margins; negative values indicate overflow or clipping.", "captures": records}, indent=2) + "\n")
                print(f"MASTER GTK AUXILIARY PASS captures={len(records)}", flush=True)
            finally:
                try:
                    app = application()
                    if app is not None:
                        (output / "application.atspi.json").write_text(json.dumps(tree(app), indent=2) + "\n")
                finally:
                    if process.poll() is None:
                        process.terminate()
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    try:
        if len(sys.argv) != 4:
            raise RuntimeError("usage: review-master-auxiliary.py BINARY ROM OUTPUT")
        review(*sys.argv[1:])
    except Exception as error:
        print(f"MASTER GTK AUXILIARY FAIL: {error}", file=sys.stderr)
        sys.exit(1)