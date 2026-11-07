import hashlib
import itertools
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import yaml

_PUSH_BUTTON = "push button"
_GEOMETRY_FORMAT = "%w %h"
_CLEAR_HISTORY = "Clear history"
_NO_NOTIFICATIONS = "No notifications"
_CREATE_SAVE_SLOT = "Create Save Slot"
_SELECT_ACTIVE_SLOT = "Select Active Slot"
_SLOT_2 = "Slot 2"
_SLOT_1 = "Slot 1"


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


class _ReviewState:
    # Mutable per-mode state threaded through the area helpers.
    def __init__(self, output, records, layout, process, window, settings_bindings, debugger_bindings):
        self.output = output
        self.records = records
        self.layout = layout
        self.process = process
        self.window = window
        self.settings_bindings = settings_bindings
        self.debugger_bindings = debugger_bindings


def _prepare_workspace(original, temporary):
    # Stage the ROM copy and settings fixture directories.
    root = Path(temporary)
    rom = root / Path(original).name
    shutil.copyfile(original, rom)
    settings = {"shared": {}, "app_state": {}, "local": {"audio": {"muted": True}, "notifications": {"status_enabled": True, "info_duration_ms": 10000}}}
    for directory in [root / "config" / "nerust", root / "settings" / "config"]:
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "settings.yaml").write_text(yaml.safe_dump(settings))
    return root, rom, settings


def _session_environment(root, accessibility):
    # Isolated environment; AT-SPI is enabled only for area reviews.
    return dict(os.environ, NO_AT_BRIDGE="0" if accessibility else "1", GTK_A11Y="atspi" if accessibility else os.environ.get("GTK_A11Y", "none"), GDK_BACKEND="x11",
               XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
               NERUST_SETTINGS_ROOT=str(root / "settings"))


def _wait_main_window(process, mode):
    def find_window(mode=mode, process=process):
        if process.poll() is not None:
            raise RuntimeError(f"{mode}: application exited; inspect log")
        result = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(process.pid)], capture_output=True, text=True, check=False)
        return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

    return wait(find_window)


def _raise_focus_size(window, width, height):
    execute("xdotool", "windowraise", window)
    execute("xdotool", "windowfocus", "--sync", window)
    execute("xdotool", "windowsize", "--sync", window, width, height)


def _inspect_basic(state, window, image, name, expected, blank):
    # Poll one screenshot until OCR and pixel checks all agree.
    execute("import", "-window", window, str(image))
    text = execute("tesseract", str(image), "stdout", "--psm", "11")
    menu_pixels = subprocess.check_output(["convert", str(image), "-crop", "400x24+0+0", "-resize", "300%", "png:-"])
    menu_text = subprocess.check_output(["tesseract", "stdin", "stdout", "--psm", "7"], input=menu_pixels).decode()
    if blank and "File" not in menu_text:
        return None
    width, height = map(int, execute("identify", "-format", _GEOMETRY_FORMAT, str(image)).split())
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


def _take_basic_snapshot(state, name, expected=None, blank=False):
    image = state.output / f"{name}.png"
    record = wait(lambda window=state.window: _inspect_basic(state, window, image, name, expected, blank))
    state.records.append(record)
    print(f"VIEW CAPTURE {name}", flush=True)


def _probe_click_action(target):
    # Probe one AT-SPI action; unimplemented methods mean "try parent",
    # while stale objects mean "retry on the next poll round".
    try:
        action = target.queryAction()
    except NotImplementedError:
        return None
    except RuntimeError:
        return None
    for index in range(action.nActions):
        if action.getName(index) in {"click", "press", "activate", "button.activate", "row.activate"}:
            return lambda action=action, index=index: action.doAction(index)
    return None


def _locate_text_action(state, app, text, prefix):
    # Find one clickable AT-SPI node or a screen fallback coordinate.
    for node in state.layout.descendants(app):
        name, requested = state.layout.normalized(node.name), state.layout.normalized(text)
        if name != requested and not (prefix and name.startswith(requested)):
            continue
        rectangle = state.layout.bounds(node)
        if rectangle is None or rectangle["width"] <= 0 or rectangle["height"] <= 0 or rectangle["x"] < 0:
            continue
        target = node
        for _ in range(3):
            probed = _probe_click_action(target)
            if probed is not None:
                return probed
            target = target.parent
            if target is None or target == app:
                break
        if node.getRoleName() in {_PUSH_BUTTON, "combo box", "menu item", "page tab"}:
            return lambda rectangle=rectangle: execute("xdotool", "mousemove", str(rectangle["x"] + rectangle["width"] // 2), str(rectangle["y"] + rectangle["height"] // 2), "click", "1")
    return None


def _click_text(state, text, prefix=False):
    def locate(state=state, text=text, prefix=prefix):
        app = state.layout.application(state.process.pid)
        if app is None:
            return None
        return _locate_text_action(state, app, text, prefix)

    perform = wait(locate)
    if perform() is False:
        raise RuntimeError(f"History Action rejected: {text}")
    print(f"VIEW HISTORY ACTION {text}", flush=True)


def _open_states(state):
    _click_text(state, "Emulation")
    _click_text(state, "Save States")


def _inspect_menu(state, name, expected, absent, active):
    # Poll the menu tree until every expected entry is reachable.
    app = state.layout.application(state.process.pid)
    if app is None:
        return None
    nodes = [node for node in state.layout.descendants(app) if state.layout.bounds(node) is not None and state.layout.bounds(node)["width"] > 0 and state.layout.bounds(node)["height"] > 0]
    labels = [state.layout.normalized(node.name) for node in nodes]
    if not all(any(label.startswith(state.layout.normalized(text)) for label in labels) for text in expected):
        return None
    if any(any(label.startswith(state.layout.normalized(text)) for label in labels) for text in absent):
        return None
    if active is not None and not any(label.startswith(state.layout.normalized(f"Slot {active}")) and "active" in label for label in labels):
        return None
    image, tree_file = state.output / f"{name}.png", state.output / f"{name}.atspi.json"
    execute("import", "-window", "root", str(image))
    tree_file.write_text(json.dumps(state.layout.tree(app), indent=2) + "\n")
    return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
            "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest(), "screen_capture": True}


def _take_menu_snapshot(state, name, expected, absent=(), active=None):
    def inspect(state=state, name=name):
        return _inspect_menu(state, name, expected, absent, active)

    try:
        state.records.append(wait(inspect))
    except Exception:
        app = state.layout.application(state.process.pid)
        if app is not None:
            (state.output / f"{name}.atspi.json").write_text(json.dumps(state.layout.tree(app), indent=2) + "\n")
            execute("import", "-window", "root", str(state.output / f"{name}.png"))
            print(f"VIEW MENU TIMEOUT {name}: " + repr([node.name for node in state.layout.descendants(app)]), flush=True)
        raise
    print(f"VIEW CAPTURE {name}", flush=True)


def _run_menu_review(state):
    _open_states(state)
    _take_menu_snapshot(state, "menu-states", [_CREATE_SAVE_SLOT, "Save Active Slot", "Load Active Slot", _SELECT_ACTIVE_SLOT, "Save Slot", "Load Slot", "Delete Slot"])
    _click_text(state, _CREATE_SAVE_SLOT)
    _open_states(state)
    _click_text(state, _CREATE_SAVE_SLOT)
    _open_states(state)
    _click_text(state, _SELECT_ACTIVE_SLOT)
    _take_menu_snapshot(state, "menu-created", [_SLOT_1, _SLOT_2], active=2)
    _click_text(state, _SLOT_1, prefix=True)
    _open_states(state)
    _click_text(state, _SELECT_ACTIVE_SLOT)
    _take_menu_snapshot(state, "menu-selected", [_SLOT_1, _SLOT_2], active=1)
    execute("xdotool", "key", "Escape", "Escape", "Escape")
    for verb in ("Save Slot", "Load Slot"):
        _open_states(state)
        _click_text(state, verb)
        _click_text(state, _SLOT_1, prefix=True)
    _open_states(state)
    _click_text(state, "Delete Slot")
    _click_text(state, _SLOT_2, prefix=True)
    _open_states(state)
    _click_text(state, _SELECT_ACTIVE_SLOT)
    _take_menu_snapshot(state, "menu-deleted", [_SLOT_1], [_SLOT_2], active=1)
    execute("xdotool", "key", "Escape", "Escape", "Escape")


def _find_history_window(process):
    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^Notification History$"], capture_output=True, text=True, check=False)
    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None


def _open_history(state):
    execute("xdotool", "windowfocus", "--sync", state.window)
    _click_text(state, "Emulation")
    _click_text(state, "Notification History", prefix=True)
    native = wait(lambda process=state.process: _find_history_window(process))
    execute("xdotool", "windowfocus", "--sync", native)
    execute("xdotool", "windowsize", "--sync", native, "592", "428")
    return native


def _inspect_history(state, native, name, expected, absent):
    # Poll the History window until its expected rows are all visible.
    if state.process.poll() is not None:
        raise RuntimeError("application exited during History review")
    frame = state.layout.window(state.process.pid, execute("xdotool", "getwindowname", native))
    if frame is None:
        return None
    nodes = [node for node in state.layout.descendants(frame) if state.layout.visible_in_frame(node, frame)]
    labels = [state.layout.normalized(node.name) for node in nodes]
    if not all(state.layout.normalized(text) in labels for text in expected) or any(state.layout.normalized(text) in labels for text in absent):
        return None
    image = state.output / f"{name}.png"
    execute("import", "-window", native, str(image))
    tree_file = state.output / f"{name}.atspi.json"
    tree_file.write_text(json.dumps(state.layout.tree(frame), indent=2) + "\n")
    close = next((node for node in nodes if node.getRoleName() == _PUSH_BUTTON and node.name == "Close"), None)
    if close is None:
        raise RuntimeError("History Close button is missing")
    outer, footer = state.layout.bounds(frame), state.layout.bounds(close)
    if footer["y"] + footer["height"] > outer["y"] + outer["height"] - 12:
        raise RuntimeError(f"History Close button has insufficient bottom margin: frame={outer}, footer={footer}")
    width, height = map(int, execute("identify", "-format", _GEOMETRY_FORMAT, str(image)).split())
    if (width, height) != (592, 428):
        raise RuntimeError(f"History size drifted: {width}x{height}")
    return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height, "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest()}


def _take_history_snapshot(state, native, name, expected, absent=()):
    image = state.output / f"{name}.png"

    def inspect(state=state, native=native):
        return _inspect_history(state, native, name, expected, absent)

    try:
        state.records.append(wait(inspect))
    except Exception:
        frame = state.layout.window(state.process.pid, execute("xdotool", "getwindowname", native))
        if frame is not None:
            execute("import", "-window", native, str(image))
            (state.output / f"{name}.atspi.json").write_text(json.dumps(state.layout.tree(frame), indent=2) + "\n")
            print(f"VIEW HISTORY TIMEOUT {name}: " + repr([node.name for node in state.layout.descendants(frame) if state.layout.visible_in_frame(node, frame)]), flush=True)
        raise
    print(f"VIEW CAPTURE {name}", flush=True)


def _reopen_history(state):
    wait(lambda process=state.process: _find_history_window(process) is None)
    return _open_history(state)


def _run_history_review(state):
    native = _open_history(state)
    _take_history_snapshot(state, native, "history-all", [_CLEAR_HISTORY, "Close", "Date/Time", "Level", "Module", "Message"])
    _click_text(state, "All")
    execute("xdotool", "key", "Home", "Down", "Down", "Return")
    _take_history_snapshot(state, native, "history-warning", ["Warning", _NO_NOTIFICATIONS, "Close"])
    _click_text(state, "Warning")
    execute("xdotool", "key", "Home", "Return")
    _take_history_snapshot(state, native, "history-restored", ["All", _CLEAR_HISTORY, "Close"], [_NO_NOTIFICATIONS])
    _click_text(state, _CLEAR_HISTORY)
    _take_history_snapshot(state, native, "history-empty", [_NO_NOTIFICATIONS, "Close"])
    _click_text(state, "Close")
    native = _reopen_history(state)
    _take_history_snapshot(state, native, "history-reopened", ["All", _NO_NOTIFICATIONS, "Close"])


def _find_debugger_window(process):
    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^Debugger$"], capture_output=True, text=True, check=False)
    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None


def _open_debugger(state, window, shortcut=False):
    execute("xdotool", "windowfocus", "--sync", window)
    if shortcut:
        execute("xdotool", "key", "F4")
    else:
        _click_text(state, "Emulation")
        _click_text(state, "Open Debugger", prefix=True)
    native = wait(lambda process=state.process: _find_debugger_window(process))
    execute("xdotool", "windowfocus", "--sync", native)
    execute("xdotool", "windowsize", "--sync", native, "1100", "946")
    return native


def _check_debugger_labels(state, visible):
    labels = {state.layout.normalized(node.name) for node in visible}
    expected = ["Registers", "Watch", "Freeze", "Bookmarks", "Memory", "Space", "Page"]
    if not all(state.layout.normalized(label) in labels for label in expected):
        return False
    return sum("0x" in node.name for node in visible if node.getRoleName() == "label") >= 3


def _collect_debugger_alignment(state, frame, visible):
    # Verify each probed row keeps its label vertically centered.
    alignment = []
    for node in visible:
        if node.getRoleName() != "label" or node.parent.getRoleName() == _PUSH_BUTTON:
            continue
        label_name = state.layout.normalized(node.name)
        if label_name not in {"space", "page", "watchnone", "freezenone", "pickaregister", "selectadumprow"} and not label_name.startswith(("disassembly", "selected", "set", "0x")):
            continue
        row = node.parent
        while row is not None and row != frame:
            if any(child.getRoleName() == _PUSH_BUTTON for child in state.layout.descendants(row)):
                break
            row = row.parent
        if row is None or row == frame:
            raise RuntimeError(f"Debugger horizontal row is missing: {node.name}")
        label_bounds, row_bounds = state.layout.bounds(node), state.layout.bounds(row)
        label_center = label_bounds["y"] + label_bounds["height"] / 2
        row_center = row_bounds["y"] + row_bounds["height"] / 2
        if abs(label_center - row_center) > 1:
            raise RuntimeError(f"Debugger label is not vertically centered: {node.name}, label={label_bounds}, row={row_bounds}")
        alignment.append({"label": node.name, "label_center_y": label_center, "row_center_y": row_center})
    if len(alignment) < 7:
        raise RuntimeError(f"Debugger alignment controls are missing: {alignment}")
    return alignment


def _check_debugger_footer(state, frame, visible):
    buttons = [node for node in visible if node.getRoleName() == _PUSH_BUTTON]
    if any(node.name == "Refresh" for node in buttons):
        raise RuntimeError("Debugger Refresh button should not exist")
    bottom = max(state.layout.bounds(node)["y"] + state.layout.bounds(node)["height"] for node in buttons)
    outer = state.layout.bounds(frame)
    if bottom > outer["y"] + outer["height"] - 12:
        raise RuntimeError(f"Debugger bottom margin is insufficient: frame={outer}, bottom={bottom}")


def _inspect_debugger(state, native, name):
    # Poll the Debugger window until its read-only layout is complete.
    frame = state.layout.window(state.process.pid, "Debugger")
    if frame is None:
        return None
    visible = [node for node in state.layout.descendants(frame) if state.layout.visible_in_frame(node, frame)]
    if not _check_debugger_labels(state, visible):
        return None
    alignment = _collect_debugger_alignment(state, frame, visible)
    image = state.output / f"{name}.png"
    tree_file = state.output / f"{name}.atspi.json"
    execute("import", "-window", native, str(image))
    tree_file.write_text(json.dumps(state.layout.tree(frame), indent=2) + "\n")
    _check_debugger_footer(state, frame, visible)
    width, height = map(int, execute("identify", "-format", _GEOMETRY_FORMAT, str(image)).split())
    if width != 1100 or (height != 946 and not (state.debugger_bindings and height <= 1150)):
        raise RuntimeError(f"Debugger size drifted: {width}x{height}")
    return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height,
            "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest(), "alignment": alignment, "scope": "read-only View layout; operation bindings not tested"}


def _take_debugger_snapshot(state, native, name):
    image = state.output / f"{name}.png"
    tree_file = state.output / f"{name}.atspi.json"

    def inspect(state=state, native=native):
        return _inspect_debugger(state, native, name)

    try:
        state.records.append(wait(inspect))
    except Exception:
        frame = state.layout.window(state.process.pid, "Debugger")
        if frame is not None:
            execute("import", "-window", native, str(image))
            tree_file.write_text(json.dumps(state.layout.tree(frame), indent=2) + "\n")
            print(f"VIEW DEBUGGER TIMEOUT {name}: " + repr([node.name for node in state.layout.descendants(frame) if state.layout.visible_in_frame(node, frame)]), flush=True)
        raise
    print(f"VIEW CAPTURE {name}", flush=True)


def _register_cells(state):
    frame = state.layout.window(state.process.pid, "Debugger")
    if frame is None:
        return None
    cells = [node for node in state.layout.descendants(frame) if node.getRoleName() == _PUSH_BUTTON and ": 0x" in node.name and state.layout.visible_in_frame(node, frame)]
    return cells if len(cells) >= 2 else None


def _select_register(state, registers, selected_index):
    import pyatspi

    register_name = registers[selected_index].name.split(":", 1)[0]
    action = registers[selected_index].queryAction()
    index = next(index for index in range(action.nActions) if action.getName(index) in {"click", "activate"})
    if not action.doAction(index):
        raise RuntimeError("Register selection was rejected")
    wait(lambda selected_index=selected_index: (cells := _register_cells(state)) and cells[selected_index].getState().contains(pyatspi.STATE_PRESSED))
    if any(cell.getState().contains(pyatspi.STATE_PRESSED) for index, cell in enumerate(_register_cells(state)) if index != selected_index):
        raise RuntimeError("Previous register selection remained active")
    return register_name


def _run_debugger_registers(state, native):
    registers = wait(lambda state=state: _register_cells(state))
    for column, selected_index in enumerate([0, (len(registers) + 1) // 2]):
        registers = wait(lambda state=state: _register_cells(state))
        register_name = _select_register(state, registers, selected_index)
        frame = state.layout.window(state.process.pid, "Debugger")
        if not any(node.name == f"Set {register_name}" for node in state.layout.descendants(frame)):
            raise RuntimeError("Register target caption did not follow selection")
        _take_debugger_snapshot(state, native, f"debugger-register-{column}")
        state.records[-1]["measured"] = {"register_accessible_content": [state.layout.bounds(cell) for cell in _register_cells(state)], "selected_register": register_name}
        state.records[-1]["scope"] = "exclusive presentation register selection and layout; domain mutation bindings not tested"


def _memory_cells(state):
    frame = state.layout.window(state.process.pid, "Debugger")
    if frame is None:
        return None
    heads = [node for node in state.layout.descendants(frame) if node.getRoleName() == _PUSH_BUTTON and node.name.lstrip("> *").strip() == "0x00000000:" and state.layout.visible_in_frame(node, frame)]
    if not heads:
        return None
    cells = [node for node in heads[0].parent if node.getRoleName() == _PUSH_BUTTON]
    return cells if len(cells) == 17 else None


def _check_memory_insets(state, cells):
    body, row = state.layout.bounds(cells[0].parent), state.layout.bounds(cells[0].parent.parent)
    row_insets = {"left": body["x"] - row["x"], "top": body["y"] - row["y"], "right": row["x"] + row["width"] - body["x"] - body["width"], "bottom": row["y"] + row["height"] - body["y"] - body["height"]}
    if row_insets != {"left": 2, "top": 2, "right": 2, "bottom": 2}:
        raise RuntimeError(f"Memory row content insets drifted: {row_insets}")
    return row_insets


def _collect_debugger_nodes(state):
    try:
        window = state.layout.window(state.process.pid, "Debugger")
        return list(state.layout.descendants(window)) if window is not None else []
    except Exception:
        return []


def _probe_jump_target(state):
    try:
        frame = state.layout.window(state.process.pid, "Debugger")
        if frame is None:
            return None
        back = next((node.name for node in state.layout.descendants(frame) if node.getRoleName() == _PUSH_BUTTON and node.name.startswith("Back (")), None)
        arrow = next((node for node in state.layout.descendants(frame) if node.getRoleName() == _PUSH_BUTTON and node.name == "\u2192" and state.layout.visible_in_frame(node, frame)), None)
        if back is None or arrow is None:
            return None
        return (back, arrow)
    except Exception:
        return None


def _run_debugger_jump(state, native, cells, row_insets):
    frame = state.layout.window(state.process.pid, "Debugger")
    targets = [state.layout.bounds(node) for node in state.layout.descendants(frame) if node.getRoleName() == _PUSH_BUTTON and node.name == "\u2192" and state.layout.visible_in_frame(node, frame)]
    if not targets:
        raise RuntimeError("NES fixture did not expose a visible disassembly target")
    back_before, arrow = wait(lambda state=state: _probe_jump_target(state))
    count_before = int("".join(character for character in back_before if character.isdigit()))
    action = arrow.queryAction()
    index = next(index for index in range(action.nActions) if action.getName(index) in {"click", "activate"})
    if not action.doAction(index):
        raise RuntimeError("Disassembly target navigation was rejected")
    wait(lambda count_before=count_before: any(node.getRoleName() == _PUSH_BUTTON and node.name == f"Back ({count_before + 1})" for node in _collect_debugger_nodes(state)))
    _take_debugger_snapshot(state, native, "debugger-jump")
    state.records[-1]["scope"] = "disassembly jump-target navigation pushes the Back stack"
    rectangles = [state.layout.bounds(cell) for cell in cells]
    gaps = [right["x"] - left["x"] - left["width"] for left, right in itertools.pairwise(rectangles)]
    if gaps != [36] * 16:
        raise RuntimeError(f"Memory accessible content gaps drifted: {gaps}")
    return rectangles, gaps, targets


def _select_memory_column(state, native, column, rectangles, gaps, row_insets, targets):
    import pyatspi

    cells = wait(lambda state=state: _memory_cells(state))
    action = cells[column].queryAction()
    index = next(index for index in range(action.nActions) if action.getName(index) in {"click", "activate"})
    if not action.doAction(index):
        raise RuntimeError("Memory cell selection was rejected")
    wait(lambda column=column: (cells := _memory_cells(state)) and cells[column].getState().contains(pyatspi.STATE_PRESSED))
    if any(cell.getState().contains(pyatspi.STATE_PRESSED) for index, cell in enumerate(_memory_cells(state)) if index != column):
        raise RuntimeError("Previous memory cell selection remained active")
    _take_debugger_snapshot(state, native, f"debugger-byte-{column}")
    state.records[-1]["measured"] = {"memory_accessible_content": rectangles, "content_gaps": gaps, "declared_column_gap": 2, "row_content_insets": row_insets, "disassembly_targets": targets, "selected_column": column, "bounds_semantics": "AT-SPI content bounds exclude button CSS padding/border; content gap is not layout gap."}
    state.records[-1]["scope"] = "presentation cell selection and layout; domain mutation bindings not tested"


def _debug_labels(state):
    # List visible Debugger labels; AT-SPI hiccups poll as empty here
    # while a dead process surfaces explicitly like the original.
    from gi.repository import GLib

    if state.process.poll() is not None:
        raise RuntimeError(f"Debugger operation exited the process: {state.process.returncode}; inspect rom.log")
    try:
        frame = state.layout.window(state.process.pid, "Debugger")
        return [node for node in state.layout.descendants(frame) if node.getRoleName() == "label" and state.layout.visible_in_frame(node, frame)] if frame is not None else []
    except GLib.Error:
        if state.process.poll() is not None:
            raise RuntimeError(f"Debugger operation exited the process: {state.process.returncode}; inspect rom.log")
        return []


def _wait_debug_text(state, text):
    return wait(lambda text=text: any(text.casefold() in node.name.casefold() for node in _debug_labels(state)))


def _locate_editable(state, label_prefix, value):
    # Find the editable field backing one Debugger row label.
    frame = state.layout.window(state.process.pid, "Debugger")
    if frame is None:
        return None
    label = next((node for node in _debug_labels(state) if node.name.casefold().startswith(label_prefix.casefold()) and node.parent.getRoleName() != _PUSH_BUTTON), None)
    if label is None:
        return None
    root = label.parent
    while root is not None and root != frame:
        for node in state.layout.descendants(root):
            try:
                editable = node.queryEditableText()
                return lambda editable=editable, value=value: editable.setTextContents(value)
            except NotImplementedError:
                pass
            except RuntimeError:
                pass
        root = root.parent
    return None


def _edit_row(state, label_prefix, value):
    if wait(lambda: _locate_editable(state, label_prefix, value))() is False:
        raise RuntimeError(f"Debugger field write rejected: {label_prefix}")


def _confirm_gone(state):
    try:
        window = state.layout.window(state.process.pid, "Debugger")
        if window is None:
            return False
        return not any(
            node.getRoleName() == _PUSH_BUTTON
            and node.name == "Confirm"
            and state.layout.visible_in_frame(node, window)
            for node in state.layout.descendants(window)
        )
    except Exception:
        return False


def _run_debugger_write_flow(state, native):
    _click_text(state, "Step Instr")
    _wait_debug_text(state, "Stepped")
    _take_debugger_snapshot(state, native, "debugger-stepped")
    _edit_row(state, "Selected", "5A")
    _click_text(state, "Write 0x", prefix=True)
    _wait_debug_text(state, "5A")
    wait(lambda: any(node.getRoleName() == _PUSH_BUTTON and node.name == "Confirm" for node in _collect_debugger_nodes(state)))
    _take_debugger_snapshot(state, native, "debugger-write-staged")
    _click_text(state, "Confirm")
    wait(lambda: (cells := _memory_cells(state)) and cells[2].name == "5A")
    _take_debugger_snapshot(state, native, "debugger-write-confirmed")
    _edit_row(state, "Selected", "6B")
    _click_text(state, "Write 0x", prefix=True)
    _wait_debug_text(state, "6B")
    _click_text(state, "Cancel")
    wait(lambda state=state: _confirm_gone(state))
    if _memory_cells(state)[2].name != "5A":
        raise RuntimeError("Cancelled debugger write mutated memory")
    _take_debugger_snapshot(state, native, "debugger-write-cancelled")


def _run_debugger_watch_flow(state, native):
    _click_text(state, "Watch")
    _wait_debug_text(state, "0x00000001: 5A")
    _take_debugger_snapshot(state, native, "debugger-watch")
    _edit_row(state, "freeze: none", "5A")
    _click_text(state, "Freeze")
    _wait_debug_text(state, "0x00000001 = 5A")
    _take_debugger_snapshot(state, native, "debugger-freeze")
    for record in state.records:
        if record["state"] in {"debugger-stepped", "debugger-write-staged", "debugger-write-confirmed", "debugger-write-cancelled", "debugger-watch", "debugger-freeze"}:
            record["scope"] = "native debugger step and memory write prepare/confirm/cancel plus watch/freeze bindings"


def _close_debugger(state):
    action = state.layout.window(state.process.pid, "Debugger").queryAction()
    index = next(index for index in range(action.nActions) if action.getName(index) == "window.close")
    if not action.doAction(index):
        raise RuntimeError("Debugger native close request was rejected")
    wait(lambda process=state.process: _find_debugger_window(process) is None)


def _run_debugger_review(state, window):
    native = _open_debugger(state, window, shortcut=state.debugger_bindings)
    _take_debugger_snapshot(state, native, "debugger-initial")
    _run_debugger_registers(state, native)
    cells = wait(lambda state=state: _memory_cells(state))
    row_insets = _check_memory_insets(state, cells)
    rectangles, gaps, targets = _run_debugger_jump(state, native, cells, row_insets)
    for column in [1, 2]:
        _select_memory_column(state, native, column, rectangles, gaps, row_insets, targets)
    if state.debugger_bindings:
        _run_debugger_write_flow(state, native)
        _run_debugger_watch_flow(state, native)
    _close_debugger(state)
    native = _open_debugger(state, window)
    _take_debugger_snapshot(state, native, "debugger-reopened")


def _find_ppu_window(process):
    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^PPU Viewer$"], capture_output=True, text=True, check=False)
    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None


def _open_ppu(state):
    execute("xdotool", "windowfocus", "--sync", state.window)
    _click_text(state, "Emulation")
    _click_text(state, "PPU Viewer", prefix=True)
    native = wait(lambda process=state.process: _find_ppu_window(process))
    execute("xdotool", "windowfocus", "--sync", native)
    execute("xdotool", "windowsize", "--sync", native, "560", "620")
    return native


def _ppu_images(state):
    frame = state.layout.window(state.process.pid, "PPU Viewer")
    if frame is None:
        return None
    images = [node for node in state.layout.descendants(frame) if node.getRoleName() == "image" and state.layout.visible_in_frame(node, frame)]
    return images or None


def _check_ppu_geometry(state, frame, name, rectangles):
    # The initial snapshot pins image geometry; hover shots reuse it.
    if name != "ppu-initial":
        return {"images": rectangles}
    outer = state.layout.bounds(frame)
    labels = {node.name: state.layout.bounds(node) for node in state.layout.descendants(frame) if node.getRoleName() == "label"}
    titles = [labels["Pattern left"], labels["Pattern right"]]
    for title, picture in zip(titles, rectangles):
        if picture["x"] - outer["x"] != 16 or picture["y"] - title["y"] - title["height"] != 12 or (picture["width"], picture["height"]) != (256, 256):
            raise RuntimeError(f"PPU image geometry drifted: title={title}, image={picture}")
    if titles[1]["y"] - rectangles[0]["y"] - rectangles[0]["height"] != 12:
        raise RuntimeError("PPU image block gap drifted")
    return {"images": rectangles, "titles": titles, "left_inset": 16, "title_image_gap": 12, "image_block_gap": 12}


def _take_ppu_snapshot(state, native, name):
    wait(lambda state=state: _ppu_images(state))
    frame = state.layout.window(state.process.pid, "PPU Viewer")
    image = state.output / f"{name}.png"
    tree_file = state.output / f"{name}.atspi.json"
    execute("import", "-window", native, str(image))
    tree_file.write_text(json.dumps(state.layout.tree(frame), indent=2) + "\n")
    width, height = map(int, execute("identify", "-format", _GEOMETRY_FORMAT, str(image)).split())
    if (width, height) != (560, 620):
        raise RuntimeError(f"PPU size drifted: {width}x{height}")
    measured = _check_ppu_geometry(state, frame, name, [state.layout.bounds(node) for node in _ppu_images(state)])
    state.records.append({"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height,
                          "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest(), "measured": measured, "scope": "NES fixture read-only image snapshot and hover; live refresh binding not tested"})
    print(f"VIEW CAPTURE {name}", flush=True)


def _close_ppu(state):
    action = state.layout.window(state.process.pid, "PPU Viewer").queryAction()
    close_index = next(index for index in range(action.nActions) if action.getName(index) == "window.close")
    if not action.doAction(close_index):
        raise RuntimeError("PPU native close request was rejected")
    wait(lambda process=state.process: _find_ppu_window(process) is None)


def _run_ppu_review(state):
    native = _open_ppu(state)
    _take_ppu_snapshot(state, native, "ppu-initial")
    picture = state.layout.bounds(wait(lambda state=state: _ppu_images(state))[0])
    outer = state.layout.bounds(state.layout.window(state.process.pid, "PPU Viewer"))
    execute("xdotool", "mousemove", "--window", native, str(picture["x"] - outer["x"] + 20), str(picture["y"] - outer["y"] + 20))
    wait(lambda: any("0x" in node.name for node in state.layout.descendants(state.layout.window(state.process.pid, "PPU Viewer")) if node.getRoleName() == "label"))
    _take_ppu_snapshot(state, native, "ppu-hover")
    execute("xdotool", "mousemove", "--window", native, "500", "500", "click", "--repeat", "12", "--delay", "50", "5")
    wait(lambda: any("== PPU flags ==" in node.name and state.layout.visible_in_frame(node, state.layout.window(state.process.pid, "PPU Viewer")) for node in state.layout.descendants(state.layout.window(state.process.pid, "PPU Viewer")) if node.getRoleName() == "label"))
    _take_ppu_snapshot(state, native, "ppu-panels")
    _close_ppu(state)
    native = _open_ppu(state)
    _take_ppu_snapshot(state, native, "ppu-reopened")


def _find_settings_window(process):
    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^nerust Settings$"], capture_output=True, text=True, check=False)
    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None


def _open_settings(state):
    execute("xdotool", "key", "ctrl+comma")
    settings_window = wait(lambda process=state.process: _find_settings_window(process))
    execute("xdotool", "windowfocus", "--sync", settings_window)
    execute("xdotool", "windowsize", "--sync", settings_window, "900", "560")
    return settings_window


def _settings_frame(state, settings_window):
    return state.layout.window(state.process.pid, execute("xdotool", "getwindowname", settings_window))


def _check_settings_footer(state, frame, visible):
    footer = [node for node in visible if node.getRoleName() == _PUSH_BUTTON and node.name in {"Cancel", "OK"}]
    if {node.name for node in footer} != {"Cancel", "OK"}:
        raise RuntimeError("Settings footer is missing controls")
    outer = state.layout.bounds(frame)
    for node in footer:
        rectangle = state.layout.bounds(node)
        if rectangle["y"] + rectangle["height"] > outer["y"] + outer["height"] - 12:
            raise RuntimeError("Settings footer does not preserve its bottom margin")


def _inspect_settings(state, settings_window, image, page, expected):
    # Poll the Settings window until its expected words are all shown.
    if state.process.poll() is not None:
        raise RuntimeError("application exited during Settings capture")
    frame = _settings_frame(state, settings_window)
    if frame is None:
        return None
    execute("import", "-window", settings_window, str(image))
    text = execute("tesseract", str(image), "stdout", "--psm", "11")
    tree_file = state.output / f"settings-{page.lower()}.atspi.json"
    tree_file.write_text(json.dumps(state.layout.tree(frame), indent=2) + "\n")
    visible = [node for node in state.layout.descendants(frame) if state.layout.visible_in_frame(node, frame)]
    labels = " ".join(node.name for node in visible)
    _check_settings_footer(state, frame, visible)
    if not all(state.layout.normalized(word) in state.layout.normalized(labels) for word in expected):
        return None
    if page == "Notifications":
        sections = [state.layout.bounds(node) for node in visible if node.name in {"Information", "Warning"}]
        if len(sections) != 2 or sections[0] == sections[1]:
            raise RuntimeError("Notifications sections overlap or are missing")
    width, height = map(int, execute("identify", "-format", _GEOMETRY_FORMAT, str(image)).split())
    if (width, height) != (900, 560):
        raise RuntimeError(f"Settings size drifted: {width}x{height}")
    return {"state": f"settings-{page.lower()}", "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height, "ocr": text, "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest()}


def _take_settings_snapshot(state, settings_window, page, expected):
    image = state.output / f"settings-{page.lower()}.png"

    def inspect(state=state, settings_window=settings_window):
        return _inspect_settings(state, settings_window, image, page, expected)

    state.records.append(wait(inspect))
    print(f"VIEW CAPTURE settings-{page.lower()}", flush=True)


def _click_page(state, settings_window, page):
    def locate(settings_window=settings_window):
        frame = _settings_frame(state, settings_window)
        return frame is not None and state.layout.select_page(frame, page)

    wait(locate)


def _click_setting(state, settings_window, command, row_label=None):
    def locate(settings_window=settings_window):
        frame = _settings_frame(state, settings_window)
        if frame is None:
            return None
        root = _setting_row_root(state, frame, row_label)
        if root is None:
            return None
        for node in state.layout.descendants(root):
            if node.getRoleName() != _PUSH_BUTTON or node.name != command:
                continue
            action = node.queryAction()
            for index in range(action.nActions):
                if action.getName(index) in {"click", "press", "activate", "button.activate"}:
                    return lambda action=action, index=index: action.doAction(index)
        return None

    if wait(locate)() is False:
        raise RuntimeError(f"Settings action was rejected: {row_label}/{command}")
    if command == "Change":
        wait(lambda: any("Press a key" in node.name and state.layout.visible_in_frame(node, _settings_frame(state, settings_window)) for node in state.layout.descendants(_settings_frame(state, settings_window))))


def _setting_row_root(state, frame, row_label):
    # Narrow the search root to the labeled row when requested.
    if row_label is None:
        return frame
    root = next((node for node in state.layout.descendants(frame) if node.getRoleName() == "label" and node.name == row_label and state.layout.visible_in_frame(node, frame)), None)
    if root is None:
        return None
    while root != frame and not any(node.getRoleName() == _PUSH_BUTTON for node in state.layout.descendants(root)):
        root = root.parent
    return root


def _locate_choice_click(state, settings_window, row_label):
    # Find the combo box click that opens one Settings choice popup.
    frame = _settings_frame(state, settings_window)
    if frame is None:
        return None
    for label in state.layout.descendants(frame):
        if label.getRoleName() != "label" or label.name != row_label or not state.layout.visible_in_frame(label, frame):
            continue
        root = label.parent
        while root is not None and root != frame:
            combo = next((node for node in state.layout.descendants(root) if node.getRoleName() == "combo box" and state.layout.visible_in_frame(node, frame)), None)
            if combo is not None:
                rectangle, outer = state.layout.bounds(combo), state.layout.bounds(frame)
                return lambda rectangle=rectangle, outer=outer, settings_window=settings_window: execute("xdotool", "mousemove", "--window", settings_window,
                    str(rectangle["x"] - outer["x"] + rectangle["width"] // 2),
                    str(rectangle["y"] - outer["y"] + rectangle["height"] // 2), "click", "1")
            root = root.parent
    return None


def _await_choice_popup(state):
    try:
        wait(lambda process=state.process: any(node.getRoleName() in {"list box", "list"} and state.layout.bounds(node) is not None and state.layout.bounds(node)["width"] > 0 for node in state.layout.descendants(state.layout.application(process.pid))))
    except Exception:
        app = state.layout.application(state.process.pid)
        (state.output / "settings-choice-timeout.atspi.json").write_text(json.dumps(state.layout.tree(app), indent=2) + "\n")
        print("VIEW CHOICE POPUP TIMEOUT " + repr([(node.name, node.getRoleName(), state.layout.bounds(node)) for node in state.layout.descendants(app) if node.getRoleName() != "label"]), flush=True)
        raise


def _change_setting_choice(state, settings_window, row_label, steps=0, last=False):
    if wait(lambda: _locate_choice_click(state, settings_window, row_label))() is False:
        raise RuntimeError(f"Settings choice rejected: {row_label}")
    _await_choice_popup(state)
    print(f"VIEW CHOICE OPEN {row_label}", flush=True)
    execute("xdotool", "key", "End" if last else "Home", *(["Down"] * steps), "Return")
    print(f"VIEW CHOICE SENT {row_label}", flush=True)


def _scroll_notifications_to_end(state, settings_window):
    frame = _settings_frame(state, settings_window)
    if frame is None:
        return False
    for node in state.layout.descendants(frame):
        if node.getRoleName() == "scroll bar" and state.layout.visible_in_frame(node, frame):
            return _scroll_bar_to_end(state, node)
    return False


def _scroll_bar_to_end(state, node):
    # Prefer the value API; a missing method falls back to a click.
    try:
        value = node.queryValue()
    except NotImplementedError:
        rectangle = state.layout.bounds(node)
        execute("xdotool", "mousemove", str(rectangle["x"] + rectangle["width"] // 2), str(rectangle["y"] + rectangle["height"] - 2), "click", "1")
        return True
    if value.maximumValue > value.minimumValue:
        return value.setCurrentValue(value.maximumValue)
    return True


def _run_settings_binding_checks(state, settings_window):
    _change_setting_choice(state, settings_window, "Player 1")
    wait(lambda: not any(node.getRoleName() == "label" and node.name == "A" and state.layout.visible_in_frame(node, _settings_frame(state, settings_window)) for node in state.layout.descendants(_settings_frame(state, settings_window))))
    _take_settings_snapshot(state, settings_window, "Controller-none", ["None", "OK"])
    _change_setting_choice(state, settings_window, "Player 1", steps=1)
    _take_settings_snapshot(state, settings_window, "Controller-restored", ["Select", "Start", "Change", "Clear", "OK"])
    _click_setting(state, settings_window, "Change", "A")
    execute("xdotool", "key", "F7")
    _take_settings_snapshot(state, settings_window, "Input-captured", ["F7", "OK"])
    _click_setting(state, settings_window, "Clear", "A")
    _take_settings_snapshot(state, settings_window, "Input-cleared", ["Unbound", "OK"])


def _run_shortcut_binding_checks(state, settings_window):
    _click_setting(state, settings_window, "Change", "Reset")
    execute("xdotool", "key", "F9")
    _take_settings_snapshot(state, settings_window, "Shortcut-captured", ["F9", "OK"])
    _click_setting(state, settings_window, "Change", "Reset")
    execute("xdotool", "key", "Escape")
    _take_settings_snapshot(state, settings_window, "Shortcut-cancelled", ["F9", "OK"])


def _reopen_settings(state, settings_window, window):
    _change_setting_choice(state, settings_window, "Filter", last=True)
    _click_setting(state, settings_window, "OK")
    wait(lambda process=state.process: _find_settings_window(process) is None)
    execute("xdotool", "windowfocus", "--sync", window)
    execute("xdotool", "key", "ctrl+comma")
    settings_window = wait(lambda process=state.process: _find_settings_window(process))
    execute("xdotool", "windowfocus", "--sync", settings_window)
    execute("xdotool", "windowsize", "--sync", settings_window, "900", "560")
    _click_page(state, settings_window, "Input")
    _click_page(state, settings_window, "Shortcuts")
    _take_settings_snapshot(state, settings_window, "Shortcut-persisted", ["F9", "OK"])
    _click_setting(state, settings_window, "Cancel")
    wait(lambda process=state.process: _find_settings_window(process) is None)


def _run_settings_review(state, window):
    settings_window = _open_settings(state)
    _take_settings_snapshot(state, settings_window, "General", ["Language", "Storage", "OK"])
    for page, expected in [("Video", ["VSync", "Scaling", "OK"]), ("Audio", ["Volume:", "Latency:", "OK"]), ("Notifications", ["Information", "Warning", "OK"])]:
        _click_page(state, settings_window, page)
        _take_settings_snapshot(state, settings_window, page, expected)
    wait(lambda: _scroll_notifications_to_end(state, settings_window))
    _take_settings_snapshot(state, settings_window, "Notifications-bottom", ["Status display", "Overlay", "OK"])
    _click_page(state, settings_window, "Input")
    _take_settings_snapshot(state, settings_window, "Input", ["Shortcuts", "Select", "Start", "Change", "Clear", "OK"])
    if state.settings_bindings:
        _run_settings_binding_checks(state, settings_window)
    _click_page(state, settings_window, "Shortcuts")
    _take_settings_snapshot(state, settings_window, "Shortcuts", ["Pause", "Reset", "Change", "Clear", "OK"])
    if state.settings_bindings:
        _run_shortcut_binding_checks(state, settings_window)
    _click_page(state, settings_window, "System")
    _take_settings_snapshot(state, settings_window, "System", ["Filter", "MMC3 IRQ Variant", "OK"])
    if state.settings_bindings:
        _reopen_settings(state, settings_window, window)
    execute("xdotool", "windowfocus", "--sync", window)


def _run_rom_flow(state, history_review, debugger_review, ppu_review, menu_review, settings_review, window):
    _take_basic_snapshot(state, "running")
    execute("xdotool", "mousemove", "--window", window, "330", "260", "click", "1")
    execute("xdotool", "key", "space")
    _take_basic_snapshot(state, "paused", expected="Paus")
    if history_review or debugger_review or ppu_review or menu_review:
        if menu_review:
            _run_menu_review(state)
        if history_review:
            _run_history_review(state)
        if debugger_review:
            _run_debugger_review(state, window)
        if ppu_review:
            _run_ppu_review(state)
        execute("xdotool", "windowfocus", "--sync", window)
    if settings_review:
        _run_settings_review(state, window)


def _dump_failure_diagnostics(state, mode):
    # Best-effort AT-SPI dump; diagnostics must never mask the error.
    try:
        app = state.layout.application(state.process.pid)
        if app is not None:
            (state.output / f"{mode}-failure.atspi.json").write_text(json.dumps(state.layout.tree(app), indent=2) + "\n")
    except Exception as diagnostic_error:
        print(f"VIEW DIAGNOSTIC UNAVAILABLE: {diagnostic_error}", flush=True)


def _run_mode(mode, arguments, binary, rom, output, records, settings, environment, flags):
    settings_review, history_review, debugger_review = flags["settings"], flags["history"], flags["debugger"]
    ppu_review, menu_review = flags["ppu"], flags["menu"]
    with (output / f"{mode}.log").open("w") as log:
        process = subprocess.Popen([binary, *arguments], env=environment, stdout=log, stderr=log)
        try:
            window = _wait_main_window(process, mode)
            _raise_focus_size(window, "660", "520")
            if flags["accessibility"]:
                import atspi_layout as layout

                state = _ReviewState(output, records, layout, process, window, flags["settings_bindings"], flags["debugger_bindings"])
            else:
                state = _ReviewState(output, records, None, process, window, False, False)
            if mode == "empty":
                _take_basic_snapshot(state, "empty", blank=True)
            else:
                _run_rom_flow(state, history_review, debugger_review, ppu_review, menu_review, settings_review, window)
            execute("xdotool", "key", "ctrl+q")
            exit_code = process.wait(timeout=15)
            if exit_code != 0:
                raise RuntimeError(f"View review shutdown failed: {exit_code}")
            return exit_code
        finally:
            if process.poll() is None:
                if flags["accessibility"]:
                    import atspi_layout as layout

                    _dump_failure_diagnostics(_ReviewState(output, records, layout, process, None, False, False), mode)
                process.terminate()
            process.wait(timeout=15)


def capture(binary, original, destination, settings_review=False, history_review=False, debugger_review=False, ppu_review=False, menu_review=False, settings_bindings=False, debugger_bindings=False):
    accessibility = settings_review or history_review or debugger_review or ppu_review or menu_review
    output = Path(destination)
    output.mkdir(parents=True, exist_ok=True)
    (output / "evidence.json").unlink(missing_ok=True)
    records = []
    shutdown_exit_codes = {}
    flags = {"settings": settings_review, "history": history_review, "debugger": debugger_review, "ppu": ppu_review, "menu": menu_review, "accessibility": accessibility, "settings_bindings": settings_bindings, "debugger_bindings": debugger_bindings}
    with tempfile.TemporaryDirectory(prefix="nerust-view-rpa-") as temporary:
        root, rom, settings = _prepare_workspace(original, temporary)
        environment = _session_environment(root, accessibility)
        for mode, arguments in [("empty", []), ("rom", [str(rom)])]:
            shutdown_exit_codes[mode] = _run_mode(mode, arguments, binary, rom, output, records, settings, environment, flags)
    (output / "evidence.json").write_text(json.dumps({"os": execute("uname", "-a"),
        "source_revision": os.environ.get("NERUST_SOURCE_REVISION"),
        "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
        "shutdown_exit_codes": shutdown_exit_codes,
        "display_geometry": execute("xdotool", "getdisplaygeometry"),
        "rom_sha256": hashlib.sha256(Path(original).read_bytes()).hexdigest(), "requested_settings_fixture": settings,
        "measurement_units": "window-relative logical pixels; PNG dimensions are raster pixels", "insets_semantics": "Parent/child accessible content distances, not inferred CSS margins or border-box allocations.",
        "captures": records}, indent=2) + "\n")
    print(f"VIEW PASS captures={len(records)}", flush=True)


if __name__ == "__main__":
    try:
        if len(sys.argv) not in (4, 5) or (len(sys.argv) == 5 and sys.argv[4] not in {"--settings", "--settings-bindings", "--history", "--debugger", "--debugger-bindings", "--ppu", "--menu"}):
            raise RuntimeError("usage: review-main.py BINARY ROM OUTPUT [--settings|--settings-bindings|--history|--debugger|--debugger-bindings|--ppu|--menu]")
        capture(*sys.argv[1:4], settings_review=bool({"--settings", "--settings-bindings"}.intersection(sys.argv[4:])), history_review="--history" in sys.argv[4:], debugger_review=bool({"--debugger", "--debugger-bindings"}.intersection(sys.argv[4:])), ppu_review="--ppu" in sys.argv[4:], menu_review="--menu" in sys.argv[4:], settings_bindings="--settings-bindings" in sys.argv[4:], debugger_bindings="--debugger-bindings" in sys.argv[4:])
    except Exception as error:
        print(f"VIEW FAIL: {error}", file=sys.stderr)
        sys.exit(1)
