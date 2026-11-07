import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import atspi_layout
from atspi_layout import bounds, descendants, in_frame, normalized, tree


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


def _validated_paths(binary, original, destination):
    # Resolve argv paths and fail closed before spawning anything.
    # The desktop binary consumes a positional ROM path, so a `--`
    # separator is not applicable; list-form Popen plus resolve and
    # existence checks are the genuine guards here.
    binary_path = Path(binary).resolve()
    if not binary_path.is_file() or not os.access(binary_path, os.X_OK):
        raise RuntimeError(f"refusing to launch non-executable binary: {binary}")
    rom_path = Path(original).resolve()
    if not rom_path.is_file():
        raise RuntimeError(f"refusing to review missing ROM: {original}")
    output = Path(destination).resolve()
    allowed = (Path.cwd().resolve(), Path(tempfile.gettempdir()).resolve())
    if not any(output == root or root in output.parents for root in allowed):
        raise RuntimeError(f"refusing to write outside workdir or tempdir: {destination}")
    return binary_path, rom_path, output


def _prepare_isolated_home(root, rom_path):
    # Stage the ROM and fixture under an isolated temporary home.
    rom = root / rom_path.name
    shutil.copyfile(rom_path, rom)
    fixture = {"shared": {}, "app_state": {}, "local": {"audio": {"muted": True}, "notifications": {"status_enabled": True, "info_duration_ms": 10000}}}
    for folder in [root / "config" / "nerust", root / "settings" / "config"]:
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "settings.yaml").write_text(json.dumps(fixture))
    return rom, fixture


def _auxiliary_environment(root):
    # Fixed AT-SPI environment for the deterministic auxiliary run.
    return dict(os.environ, NO_AT_BRIDGE="0", GTK_A11Y="atspi", GDK_BACKEND="x11", XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"), NERUST_SETTINGS_ROOT=str(root / "settings"))


def _find_window(process, pattern):
    # Poll xdotool for a visible window owned by the review process.
    if process.poll() is not None:
        raise RuntimeError("master exited; inspect application.log")
    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", pattern], capture_output=True, text=True, check=False)
    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None


def _focus(window):
    execute("xdotool", "windowraise", window)
    execute("xdotool", "windowfocus", "--sync", window)


def _accessible_frame(process, window):
    return atspi_layout.window(process.pid, execute("xdotool", "getwindowname", window))


def _snapshot_record(process, window, output, name, expected):
    # Capture one PNG plus AT-SPI dump once all expected labels appear.
    image = output / f"{name}.png"

    def inspect():
        frame = _accessible_frame(process, window)
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

    return wait(inspect, f"capture {name}: {expected}")


def _snapshot(process, window, output, records, name, expected):
    record = _snapshot_record(process, window, output, name, expected)
    records.append(record)
    print(f"MASTER GTK CAPTURE {name}", flush=True)


def _probe_click(target):
    # Probe AT-SPI actions; unimplemented methods mean "try parent".
    # Stale objects mean "retry on next poll round" via the same path.
    try:
        action = target.queryAction()
    except NotImplementedError:
        return None
    except RuntimeError:
        return None
    for index in range(action.nActions):
        if action.getName(index) in {"click", "press", "activate", "button.activate", "row.activate"}:
            return lambda action=action, index=index: action.doAction(index), target.name, action.getName(index)
    return None


def _probe_select(target):
    # Probe AT-SPI selection; unimplemented or stale means "try parent".
    try:
        selection = target.parent.querySelection()
        index = target.getIndexInParent()
        return lambda selection=selection, index=index: selection.selectChild(index), target.name, "selectChild"
    except NotImplementedError:
        return None
    except RuntimeError:
        return None


def _locate_clickable(process, window, text, sidebar):
    frame = _accessible_frame(process, window)
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
            probed = _probe_click(target)
            if probed is not None:
                return probed
            if sidebar and target.parent is not None:
                selected = _probe_select(target)
                if selected is not None:
                    return selected
            target = target.parent
            if target is None or target == frame:
                break
    return None


def _click_text(process, window, text, sidebar=False):
    perform, target_name, action_name = wait(lambda: _locate_clickable(process, window, text, sidebar), f"AT-SPI Action {text} in window {window}")
    print(f"MASTER GTK ACTION {text}: {target_name} / {action_name}", flush=True)
    if not perform():
        raise RuntimeError(f"AT-SPI Action rejected: {text}")


def _window_action(process, window, name):
    frame = wait(lambda: _accessible_frame(process, window), f"accessible window for {name}")
    action = frame.queryAction()
    for index in range(action.nActions):
        if action.getName(index) == name:
            print(f"MASTER GTK WINDOW ACTION {name}", flush=True)
            if not action.doAction(index):
                raise RuntimeError(f"AT-SPI window Action rejected: {name}")
            return
    raise RuntimeError(f"AT-SPI window Action missing: {name}")


def _capture_settings(process, output, records):
    settings = wait(lambda: _find_window(process, "Preferences|Settings"), "Settings native window after Action")
    _focus(settings)
    _snapshot(process, settings, output, records, "settings-general", ["Language", "Storage", "Cancel"])
    for page, expected in [("Input", ["Input", "Cancel"]), ("Video", ["Scaling", "Cancel"]), ("Audio", ["Latency", "Cancel"]), ("Notifications", ["Warning", "Cancel"]), ("System", ["System", "Cancel"])]:
        _click_text(process, settings, page, sidebar=True)
        _snapshot(process, settings, output, records, f"settings-{page.lower()}", expected)
    execute("xdotool", "key", "Escape")


def _capture_debugger(process, output, records):
    debugger = wait(lambda: _find_window(process, "Debugger"), "Debugger native window")
    _focus(debugger)
    _snapshot(process, debugger, output, records, "debugger", ["Step", "PPU"])
    _click_text(process, debugger, "PPU Viewer")
    ppu = wait(lambda: _find_window(process, "PPU"), "PPU native window")
    _focus(ppu)
    _snapshot(process, ppu, output, records, "ppu", ["Pattern"])


def _capture_history(process, output, records, main):
    _focus(main)
    _window_action(process, main, "win.history")
    history = wait(lambda: _find_window(process, "Notification [Hh]istory"), "History native window")
    _focus(history)
    _snapshot(process, history, output, records, "history", ["Level", "Clear", "Close"])


def _write_evidence(binary_path, rom_path, output, records, fixture):
    # Emit the deterministic evidence document for this run.
    (output / "evidence.json").write_text(json.dumps({"os": execute("uname", "-a"), "source_revision": os.environ.get("NERUST_SOURCE_REVISION"), "binary_sha256": hashlib.sha256(binary_path.read_bytes()).hexdigest(), "display_geometry": execute("xdotool", "getdisplaygeometry"), "rom_sha256": hashlib.sha256(rom_path.read_bytes()).hexdigest(), "requested_settings_fixture": fixture, "measurement_units": "screen pixels", "insets_semantics": "Distances between accessible parent and child allocations, not inferred CSS margins; negative values indicate overflow or clipping.", "captures": records}, indent=2) + "\n")
    print(f"MASTER GTK AUXILIARY PASS captures={len(records)}", flush=True)


def _dump_diagnostics(process, output):
    try:
        app = atspi_layout.application(process.pid)
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


def _run_session(binary_path, rom, output, records, fixture, root, log):
    process = subprocess.Popen([binary_path, str(rom)], env=_auxiliary_environment(root), stdout=log, stderr=log)
    try:
        main = wait(lambda: _find_window(process, "."), "main native window owned by process")
        _focus(main)
        execute("xdotool", "windowsize", "--sync", main, "660", "520")
        _window_action(process, main, "win.settings")
        _capture_settings(process, output, records)
        _focus(main)
        _window_action(process, main, "win.debugger")
        _capture_debugger(process, output, records)
        _capture_history(process, output, records, main)
        _write_evidence(binary_path, rom, output, records, fixture)
    finally:
        _dump_diagnostics(process, output)


def review(binary, original, destination):
    binary_path, rom_path, output = _validated_paths(binary, original, destination)
    output.mkdir(parents=True, exist_ok=True)
    report = output / "evidence.json"
    report.unlink(missing_ok=True)
    records = []
    with tempfile.TemporaryDirectory(prefix="nerust-master-auxiliary-") as temporary:
        root = Path(temporary)
        rom, fixture = _prepare_isolated_home(root, rom_path)
        with (output / "application.log").open("w") as log:
            _run_session(binary_path, rom, output, records, fixture, root, log)


if __name__ == "__main__":
    try:
        if len(sys.argv) != 4:
            raise RuntimeError("usage: review-master-auxiliary.py BINARY ROM OUTPUT")
        review(*sys.argv[1:])
    except Exception as error:
        print(f"MASTER GTK AUXILIARY FAIL: {error}", file=sys.stderr)
        sys.exit(1)
