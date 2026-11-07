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


def capture(binary, original, destination, settings_review=False, history_review=False, debugger_review=False, ppu_review=False, menu_review=False, settings_bindings=False, debugger_bindings=False):
    accessibility = settings_review or history_review or debugger_review or ppu_review or menu_review
    if accessibility:
        import atspi_layout as layout
    output = Path(destination)
    output.mkdir(parents=True, exist_ok=True)
    (output / "evidence.json").unlink(missing_ok=True)
    records = []
    shutdown_exit_codes = {}
    with tempfile.TemporaryDirectory(prefix="nerust-view-rpa-") as temporary:
        root = Path(temporary)
        rom = root / Path(original).name
        shutil.copyfile(original, rom)
        settings = {"shared": {}, "app_state": {}, "local": {"audio": {"muted": True}, "notifications": {"status_enabled": True, "info_duration_ms": 10000}}}
        for directory in [root / "config" / "nerust", root / "settings" / "config"]:
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "settings.yaml").write_text(yaml.safe_dump(settings))
        environment = dict(os.environ, NO_AT_BRIDGE="0" if accessibility else "1", GTK_A11Y="atspi" if accessibility else os.environ.get("GTK_A11Y", "none"), GDK_BACKEND="x11",
                           XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
                           NERUST_SETTINGS_ROOT=str(root / "settings"))
        for mode, arguments in [("empty", []), ("rom", [str(rom)])]:
            with (output / f"{mode}.log").open("w") as log:
                process = subprocess.Popen([binary, *arguments], env=environment, stdout=log, stderr=log)
                try:
                    def find_window(mode=mode, process=process):
                        if process.poll() is not None:
                            raise RuntimeError(f"{mode}: application exited; inspect log")
                        result = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(process.pid)], capture_output=True, text=True, check=False)
                        return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                    window = wait(find_window)
                    execute("xdotool", "windowraise", window)
                    execute("xdotool", "windowfocus", "--sync", window)
                    execute("xdotool", "windowsize", "--sync", window, "660", "520")

                    def snapshot(name, expected=None, blank=False, process=process, window=window):
                        image = output / f"{name}.png"

                        def inspect(window=window, process=process):
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
                        if history_review or debugger_review or ppu_review or menu_review:
                            def click_text(text, prefix=False, process=process):
                                def locate(process=process):
                                    app = layout.application(process.pid)
                                    if app is None:
                                        return None
                                    for node in layout.descendants(app):
                                        name, requested = layout.normalized(node.name), layout.normalized(text)
                                        if name != requested and not (prefix and name.startswith(requested)):
                                            continue
                                        rectangle = layout.bounds(node)
                                        if rectangle is None or rectangle["width"] <= 0 or rectangle["height"] <= 0 or rectangle["x"] < 0:
                                            continue
                                        target = node
                                        for _ in range(3):
                                            try:
                                                action = target.queryAction()
                                                for index in range(action.nActions):
                                                    if action.getName(index) in {"click", "press", "activate", "button.activate", "row.activate"}:
                                                        return lambda action=action, index=index: action.doAction(index)
                                            except (NotImplementedError, RuntimeError):
                                                pass
                                            target = target.parent
                                            if target is None or target == app:
                                                break
                                        if node.getRoleName() in {"push button", "combo box", "menu item", "page tab"}:
                                            return lambda rectangle=rectangle: execute("xdotool", "mousemove", str(rectangle["x"] + rectangle["width"] // 2), str(rectangle["y"] + rectangle["height"] // 2), "click", "1")
                                    return None
                                perform = wait(locate)
                                if perform() is False:
                                    raise RuntimeError(f"History Action rejected: {text}")
                                print(f"VIEW HISTORY ACTION {text}", flush=True)

                            if menu_review:
                                def open_states():
                                    click_text("Emulation")
                                    click_text("Save States")

                                def menu_snapshot(name, expected, absent=(), active=None, process=process):
                                    def inspect(process=process):
                                        app = layout.application(process.pid)
                                        if app is None:
                                            return None
                                        nodes = [node for node in layout.descendants(app) if layout.bounds(node) is not None and layout.bounds(node)["width"] > 0 and layout.bounds(node)["height"] > 0]
                                        labels = [layout.normalized(node.name) for node in nodes]
                                        if not all(any(label.startswith(layout.normalized(text)) for label in labels) for text in expected):
                                            return None
                                        if any(any(label.startswith(layout.normalized(text)) for label in labels) for text in absent):
                                            return None
                                        if active is not None and not any(label.startswith(layout.normalized(f"Slot {active}")) and "active" in label for label in labels):
                                            return None
                                        image, tree_file = output / f"{name}.png", output / f"{name}.atspi.json"
                                        execute("import", "-window", "root", str(image))
                                        tree_file.write_text(json.dumps(layout.tree(app), indent=2) + "\n")
                                        return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
                                                "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest(), "screen_capture": True}
                                    try:
                                        records.append(wait(inspect))
                                    except Exception:
                                        app = layout.application(process.pid)
                                        if app is not None:
                                            (output / f"{name}.atspi.json").write_text(json.dumps(layout.tree(app), indent=2) + "\n")
                                            execute("import", "-window", "root", str(output / f"{name}.png"))
                                            print(f"VIEW MENU TIMEOUT {name}: " + repr([node.name for node in layout.descendants(app)]), flush=True)
                                        raise
                                    print(f"VIEW CAPTURE {name}", flush=True)

                                open_states()
                                menu_snapshot("menu-states", ["Create Save Slot", "Save Active Slot", "Load Active Slot", "Select Active Slot", "Save Slot", "Load Slot", "Delete Slot"])
                                click_text("Create Save Slot")
                                open_states()
                                click_text("Create Save Slot")
                                open_states()
                                click_text("Select Active Slot")
                                menu_snapshot("menu-created", ["Slot 1", "Slot 2"], active=2)
                                click_text("Slot 1", prefix=True)
                                open_states()
                                click_text("Select Active Slot")
                                menu_snapshot("menu-selected", ["Slot 1", "Slot 2"], active=1)
                                execute("xdotool", "key", "Escape", "Escape", "Escape")
                                for verb in ("Save Slot", "Load Slot"):
                                    open_states()
                                    click_text(verb)
                                    click_text("Slot 1", prefix=True)
                                open_states()
                                click_text("Delete Slot")
                                click_text("Slot 2", prefix=True)
                                open_states()
                                click_text("Select Active Slot")
                                menu_snapshot("menu-deleted", ["Slot 1"], ["Slot 2"], active=1)
                                execute("xdotool", "key", "Escape", "Escape", "Escape")

                            def find_history(process=process):
                                result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^Notification History$"], capture_output=True, text=True, check=False)
                                return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                            def open_history(window=window):
                                execute("xdotool", "windowfocus", "--sync", window)
                                click_text("Emulation")
                                click_text("Notification History", prefix=True)
                                native = wait(find_history)
                                execute("xdotool", "windowfocus", "--sync", native)
                                execute("xdotool", "windowsize", "--sync", native, "592", "428")
                                return native

                            def history_snapshot(native, name, expected, absent=(), process=process):
                                image = output / f"{name}.png"
                                def inspect(process=process):
                                    if process.poll() is not None:
                                        raise RuntimeError("application exited during History review")
                                    frame = layout.window(process.pid, execute("xdotool", "getwindowname", native))
                                    if frame is None:
                                        return None
                                    nodes = [node for node in layout.descendants(frame) if layout.visible_in_frame(node, frame)]
                                    labels = [layout.normalized(node.name) for node in nodes]
                                    if not all(layout.normalized(text) in labels for text in expected) or any(layout.normalized(text) in labels for text in absent):
                                        return None
                                    execute("import", "-window", native, str(image))
                                    tree_file = output / f"{name}.atspi.json"
                                    tree_file.write_text(json.dumps(layout.tree(frame), indent=2) + "\n")
                                    close = next((node for node in nodes if node.getRoleName() == "push button" and node.name == "Close"), None)
                                    if close is None:
                                        raise RuntimeError("History Close button is missing")
                                    outer, footer = layout.bounds(frame), layout.bounds(close)
                                    if footer["y"] + footer["height"] > outer["y"] + outer["height"] - 12:
                                        raise RuntimeError(f"History Close button has insufficient bottom margin: frame={outer}, footer={footer}")
                                    width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                                    if (width, height) != (592, 428):
                                        raise RuntimeError(f"History size drifted: {width}x{height}")
                                    return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height, "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest()}
                                try:
                                    records.append(wait(inspect))
                                except Exception:
                                    frame = layout.window(process.pid, execute("xdotool", "getwindowname", native))
                                    if frame is not None:
                                        execute("import", "-window", native, str(image))
                                        (output / f"{name}.atspi.json").write_text(json.dumps(layout.tree(frame), indent=2) + "\n")
                                        print(f"VIEW HISTORY TIMEOUT {name}: " + repr([node.name for node in layout.descendants(frame) if layout.visible_in_frame(node, frame)]), flush=True)
                                    raise
                                print(f"VIEW CAPTURE {name}", flush=True)

                            if history_review:
                                native = open_history()
                                history_snapshot(native, "history-all", ["Clear history", "Close", "Date/Time", "Level", "Module", "Message"])
                                click_text("All")
                                execute("xdotool", "key", "Home", "Down", "Down", "Return")
                                history_snapshot(native, "history-warning", ["Warning", "No notifications", "Close"])
                                click_text("Warning")
                                execute("xdotool", "key", "Home", "Return")
                                history_snapshot(native, "history-restored", ["All", "Clear history", "Close"], ["No notifications"])
                                click_text("Clear history")
                                history_snapshot(native, "history-empty", ["No notifications", "Close"])
                                click_text("Close")
                                wait(lambda: find_history() is None)
                                native = open_history()
                                history_snapshot(native, "history-reopened", ["All", "No notifications", "Close"])
                            if debugger_review:
                                def find_debugger(process=process):
                                    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^Debugger$"], capture_output=True, text=True, check=False)
                                    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                                def open_debugger(window, shortcut=False):
                                    execute("xdotool", "windowfocus", "--sync", window)
                                    if shortcut:
                                        execute("xdotool", "key", "F4")
                                    else:
                                        click_text("Emulation")
                                        click_text("Open Debugger", prefix=True)
                                    native = wait(find_debugger)
                                    execute("xdotool", "windowfocus", "--sync", native)
                                    execute("xdotool", "windowsize", "--sync", native, "1100", "946")
                                    return native

                                def debugger_snapshot(native, name, process=process):
                                    image = output / f"{name}.png"
                                    tree_file = output / f"{name}.atspi.json"
                                    def inspect(process=process):
                                        frame = layout.window(process.pid, "Debugger")
                                        if frame is None:
                                            return None
                                        visible = [node for node in layout.descendants(frame) if layout.visible_in_frame(node, frame)]
                                        labels = {layout.normalized(node.name) for node in visible}
                                        expected = ["Registers", "Watch", "Freeze", "Bookmarks", "Memory", "Space", "Page"]
                                        if not all(layout.normalized(label) in labels for label in expected):
                                            return None
                                        if sum("0x" in node.name for node in visible if node.getRoleName() == "label") < 3:
                                            return None
                                        alignment = []
                                        for node in visible:
                                            if node.getRoleName() != "label" or node.parent.getRoleName() == "push button":
                                                continue
                                            label_name = layout.normalized(node.name)
                                            if label_name not in {"space", "page", "watchnone", "freezenone", "pickaregister", "selectadumprow"} and not label_name.startswith(("disassembly", "selected", "set", "0x")):
                                                continue
                                            row = node.parent
                                            while row is not None and row != frame:
                                                if any(child.getRoleName() == "push button" for child in layout.descendants(row)):
                                                    break
                                                row = row.parent
                                            if row is None or row == frame:
                                                raise RuntimeError(f"Debugger horizontal row is missing: {node.name}")
                                            label_bounds, row_bounds = layout.bounds(node), layout.bounds(row)
                                            label_center = label_bounds["y"] + label_bounds["height"] / 2
                                            row_center = row_bounds["y"] + row_bounds["height"] / 2
                                            if abs(label_center - row_center) > 1:
                                                raise RuntimeError(f"Debugger label is not vertically centered: {node.name}, label={label_bounds}, row={row_bounds}")
                                            alignment.append({"label": node.name, "label_center_y": label_center, "row_center_y": row_center})
                                        if len(alignment) < 7:
                                            raise RuntimeError(f"Debugger alignment controls are missing: {alignment}")
                                        execute("import", "-window", native, str(image))
                                        tree_file.write_text(json.dumps(layout.tree(frame), indent=2) + "\n")
                                        buttons = [node for node in visible if node.getRoleName() == "push button"]
                                        if any(node.name == "Refresh" for node in buttons):
                                            raise RuntimeError("Debugger Refresh button should not exist")
                                        bottom = max(layout.bounds(node)["y"] + layout.bounds(node)["height"] for node in buttons)
                                        outer = layout.bounds(frame)
                                        if bottom > outer["y"] + outer["height"] - 12:
                                            raise RuntimeError(f"Debugger bottom margin is insufficient: frame={outer}, bottom={bottom}")
                                        width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                                        if width != 1100 or (height != 946 and not (debugger_bindings and height <= 1150)):
                                            raise RuntimeError(f"Debugger size drifted: {width}x{height}")
                                        return {"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height,
                                                "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest(), "alignment": alignment, "scope": "read-only View layout; operation bindings not tested"}
                                    try:
                                        records.append(wait(inspect))
                                    except Exception:
                                        frame = layout.window(process.pid, "Debugger")
                                        if frame is not None:
                                            execute("import", "-window", native, str(image))
                                            tree_file.write_text(json.dumps(layout.tree(frame), indent=2) + "\n")
                                            print(f"VIEW DEBUGGER TIMEOUT {name}: " + repr([node.name for node in layout.descendants(frame) if layout.visible_in_frame(node, frame)]), flush=True)
                                        raise
                                    print(f"VIEW CAPTURE {name}", flush=True)

                                native = open_debugger(window, shortcut=debugger_bindings)
                                debugger_snapshot(native, "debugger-initial")
                                import pyatspi

                                def register_cells(process=process):
                                    frame = layout.window(process.pid, "Debugger")
                                    if frame is None:
                                        return None
                                    cells = [node for node in layout.descendants(frame) if node.getRoleName() == "push button" and ": 0x" in node.name and layout.visible_in_frame(node, frame)]
                                    return cells if len(cells) >= 2 else None

                                registers = wait(register_cells)
                                for column, selected_index in enumerate([0, (len(registers) + 1) // 2]):
                                    registers = wait(register_cells)
                                    register_name = registers[selected_index].name.split(":", 1)[0]
                                    action = registers[selected_index].queryAction()
                                    index = next(index for index in range(action.nActions) if action.getName(index) in {"click", "activate"})
                                    if not action.doAction(index):
                                        raise RuntimeError("Register selection was rejected")
                                    wait(lambda selected_index=selected_index: (cells := register_cells()) and cells[selected_index].getState().contains(pyatspi.STATE_PRESSED))
                                    if any(cell.getState().contains(pyatspi.STATE_PRESSED) for index, cell in enumerate(register_cells()) if index != selected_index):
                                        raise RuntimeError("Previous register selection remained active")
                                    frame = layout.window(process.pid, "Debugger")
                                    if not any(node.name == f"Set {register_name}" for node in layout.descendants(frame)):
                                        raise RuntimeError("Register target caption did not follow selection")
                                    debugger_snapshot(native, f"debugger-register-{column}")
                                    records[-1]["measured"] = {"register_accessible_content": [layout.bounds(cell) for cell in register_cells()], "selected_register": register_name}
                                    records[-1]["scope"] = "exclusive presentation register selection and layout; domain mutation bindings not tested"

                                def memory_cells(process=process):
                                    frame = layout.window(process.pid, "Debugger")
                                    if frame is None:
                                        return None
                                    heads = [node for node in layout.descendants(frame) if node.getRoleName() == "push button" and node.name.lstrip("> *").strip() == "0x00000000:" and layout.visible_in_frame(node, frame)]
                                    if not heads:
                                        return None
                                    cells = [node for node in heads[0].parent if node.getRoleName() == "push button"]
                                    return cells if len(cells) == 17 else None

                                cells = wait(memory_cells)
                                body, row = layout.bounds(cells[0].parent), layout.bounds(cells[0].parent.parent)
                                row_insets = {"left": body["x"] - row["x"], "top": body["y"] - row["y"], "right": row["x"] + row["width"] - body["x"] - body["width"], "bottom": row["y"] + row["height"] - body["y"] - body["height"]}
                                if row_insets != {"left": 2, "top": 2, "right": 2, "bottom": 2}:
                                    raise RuntimeError(f"Memory row content insets drifted: {row_insets}")
                                frame = layout.window(process.pid, "Debugger")
                                targets = [layout.bounds(node) for node in layout.descendants(frame) if node.getRoleName() == "push button" and node.name == "\u2192" and layout.visible_in_frame(node, frame)]
                                if not targets:
                                    raise RuntimeError("NES fixture did not expose a visible disassembly target")
                                def debugger_nodes(process=process):
                                    try:
                                        window = layout.window(process.pid, "Debugger")
                                        return list(layout.descendants(window)) if window is not None else []
                                    except Exception:
                                        return []
                                # Jump-target navigation: clicking the first
                                # visible arrow must push the Back stack.
                                def jump_target(process=process):
                                    try:
                                        frame = layout.window(process.pid, "Debugger")
                                        if frame is None:
                                            return None
                                        back = next((node.name for node in layout.descendants(frame) if node.getRoleName() == "push button" and node.name.startswith("Back (")), None)
                                        arrow = next((node for node in layout.descendants(frame) if node.getRoleName() == "push button" and node.name == "\u2192" and layout.visible_in_frame(node, frame)), None)
                                        if back is None or arrow is None:
                                            return None
                                        return (back, arrow)
                                    except Exception:
                                        return None
                                back_before, arrow = wait(jump_target)
                                count_before = int("".join(character for character in back_before if character.isdigit()))
                                action = arrow.queryAction()
                                index = next(index for index in range(action.nActions) if action.getName(index) in {"click", "activate"})
                                if not action.doAction(index):
                                    raise RuntimeError("Disassembly target navigation was rejected")
                                wait(lambda count_before=count_before: any(node.getRoleName() == "push button" and node.name == f"Back ({count_before + 1})" for node in debugger_nodes()))
                                debugger_snapshot(native, "debugger-jump")
                                records[-1]["scope"] = "disassembly jump-target navigation pushes the Back stack"
                                rectangles = [layout.bounds(cell) for cell in cells]
                                gaps = [right["x"] - left["x"] - left["width"] for left, right in itertools.pairwise(rectangles)]
                                if gaps != [36] * 16:
                                    raise RuntimeError(f"Memory accessible content gaps drifted: {gaps}")
                                for column in [1, 2]:
                                    cells = wait(memory_cells)
                                    action = cells[column].queryAction()
                                    index = next(index for index in range(action.nActions) if action.getName(index) in {"click", "activate"})
                                    if not action.doAction(index):
                                        raise RuntimeError("Memory cell selection was rejected")
                                    wait(lambda column=column: (cells := memory_cells()) and cells[column].getState().contains(pyatspi.STATE_PRESSED))
                                    if any(cell.getState().contains(pyatspi.STATE_PRESSED) for index, cell in enumerate(memory_cells()) if index != column):
                                        raise RuntimeError("Previous memory cell selection remained active")
                                    debugger_snapshot(native, f"debugger-byte-{column}")
                                    records[-1]["measured"] = {"memory_accessible_content": rectangles, "content_gaps": gaps, "declared_column_gap": 2, "row_content_insets": row_insets, "disassembly_targets": targets, "selected_column": column, "bounds_semantics": "AT-SPI content bounds exclude button CSS padding/border; content gap is not layout gap."}
                                    records[-1]["scope"] = "presentation cell selection and layout; domain mutation bindings not tested"
                                if debugger_bindings:
                                    from gi.repository import GLib

                                    def debug_labels(process=process):
                                        if process.poll() is not None:
                                            raise RuntimeError(f"Debugger operation exited the process: {process.returncode}; inspect rom.log")
                                        try:
                                            frame = layout.window(process.pid, "Debugger")
                                            return [node for node in layout.descendants(frame) if node.getRoleName() == "label" and layout.visible_in_frame(node, frame)] if frame is not None else []
                                        except GLib.Error:
                                            if process.poll() is not None:
                                                raise RuntimeError(f"Debugger operation exited the process: {process.returncode}; inspect rom.log")
                                            return []

                                    def wait_debug_text(text):
                                        return wait(lambda: any(text.casefold() in node.name.casefold() for node in debug_labels()))

                                    def edit_row(label_prefix, value, process=process):
                                        def locate(process=process):
                                            frame = layout.window(process.pid, "Debugger")
                                            if frame is None:
                                                return None
                                            label = next((node for node in debug_labels() if node.name.casefold().startswith(label_prefix.casefold()) and node.parent.getRoleName() != "push button"), None)
                                            if label is None:
                                                return None
                                            root = label.parent
                                            while root is not None and root != frame:
                                                for node in layout.descendants(root):
                                                    try:
                                                        editable = node.queryEditableText()
                                                        return lambda editable=editable: editable.setTextContents(value)
                                                    except (NotImplementedError, RuntimeError):
                                                        pass
                                                root = root.parent
                                            return None
                                        if wait(locate)() is False:
                                            raise RuntimeError(f"Debugger field write rejected: {label_prefix}")

                                    click_text("Step Instr")
                                    wait_debug_text("Stepped")
                                    debugger_snapshot(native, "debugger-stepped")
                                    edit_row("Selected", "5A")
                                    click_text("Write 0x", prefix=True)
                                    wait_debug_text("5A")
                                    wait(lambda: any(node.getRoleName() == "push button" and node.name == "Confirm" for node in debugger_nodes()))
                                    debugger_snapshot(native, "debugger-write-staged")
                                    click_text("Confirm")
                                    wait(lambda: (cells := memory_cells()) and cells[2].name == "5A")
                                    debugger_snapshot(native, "debugger-write-confirmed")
                                    edit_row("Selected", "6B")
                                    click_text("Write 0x", prefix=True)
                                    wait_debug_text("6B")
                                    click_text("Cancel")
                                    def confirm_gone(process=process):
                                        try:
                                            window = layout.window(process.pid, "Debugger")
                                            if window is None:
                                                return False
                                            return not any(
                                                node.getRoleName() == "push button"
                                                and node.name == "Confirm"
                                                and layout.visible_in_frame(node, window)
                                                for node in layout.descendants(window)
                                            )
                                        except Exception:
                                            return False

                                    wait(confirm_gone)
                                    if memory_cells()[2].name != "5A":
                                        raise RuntimeError("Cancelled debugger write mutated memory")
                                    debugger_snapshot(native, "debugger-write-cancelled")
                                    click_text("Watch")
                                    wait_debug_text("0x00000001: 5A")
                                    debugger_snapshot(native, "debugger-watch")
                                    edit_row("freeze: none", "5A")
                                    click_text("Freeze")
                                    wait_debug_text("0x00000001 = 5A")
                                    debugger_snapshot(native, "debugger-freeze")
                                    for record in records:
                                        if record["state"] in {"debugger-stepped", "debugger-write-staged", "debugger-write-confirmed", "debugger-write-cancelled", "debugger-watch", "debugger-freeze"}:
                                            record["scope"] = "native debugger step and memory write prepare/confirm/cancel plus watch/freeze bindings"
                                action = layout.window(process.pid, "Debugger").queryAction()
                                index = next(index for index in range(action.nActions) if action.getName(index) == "window.close")
                                if not action.doAction(index):
                                    raise RuntimeError("Debugger native close request was rejected")
                                wait(lambda: find_debugger() is None)
                                native = open_debugger(window)
                                debugger_snapshot(native, "debugger-reopened")
                            if ppu_review:
                                def find_ppu(process=process):
                                    result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^PPU Viewer$"], capture_output=True, text=True, check=False)
                                    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                                def open_ppu(window=window):
                                    execute("xdotool", "windowfocus", "--sync", window)
                                    click_text("Emulation")
                                    click_text("PPU Viewer", prefix=True)
                                    native = wait(find_ppu)
                                    execute("xdotool", "windowfocus", "--sync", native)
                                    execute("xdotool", "windowsize", "--sync", native, "560", "620")
                                    return native

                                def ppu_frame(process=process):
                                    return layout.window(process.pid, "PPU Viewer")

                                def ppu_images():
                                    frame = ppu_frame()
                                    if frame is None:
                                        return None
                                    images = [node for node in layout.descendants(frame) if node.getRoleName() == "image" and layout.visible_in_frame(node, frame)]
                                    return images or None

                                def ppu_snapshot(native, name):
                                    wait(ppu_images)
                                    frame = ppu_frame()
                                    image = output / f"{name}.png"
                                    tree_file = output / f"{name}.atspi.json"
                                    execute("import", "-window", native, str(image))
                                    tree_file.write_text(json.dumps(layout.tree(frame), indent=2) + "\n")
                                    width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                                    if (width, height) != (560, 620):
                                        raise RuntimeError(f"PPU size drifted: {width}x{height}")
                                    rectangles = [layout.bounds(node) for node in ppu_images()]
                                    measured = {"images": rectangles}
                                    if name == "ppu-initial":
                                        outer = layout.bounds(frame)
                                        labels = {node.name: layout.bounds(node) for node in layout.descendants(frame) if node.getRoleName() == "label"}
                                        titles = [labels["Pattern left"], labels["Pattern right"]]
                                        for title, picture in zip(titles, rectangles):
                                            if picture["x"] - outer["x"] != 16 or picture["y"] - title["y"] - title["height"] != 12 or (picture["width"], picture["height"]) != (256, 256):
                                                raise RuntimeError(f"PPU image geometry drifted: title={title}, image={picture}")
                                        if titles[1]["y"] - rectangles[0]["y"] - rectangles[0]["height"] != 12:
                                            raise RuntimeError("PPU image block gap drifted")
                                        measured["titles"] = titles
                                        measured["left_inset"] = 16
                                        measured["title_image_gap"] = 12
                                        measured["image_block_gap"] = 12
                                    records.append({"state": name, "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height,
                                                    "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest(), "measured": measured, "scope": "NES fixture read-only image snapshot and hover; live refresh binding not tested"})
                                    print(f"VIEW CAPTURE {name}", flush=True)

                                native = open_ppu()
                                ppu_snapshot(native, "ppu-initial")
                                picture = layout.bounds(wait(ppu_images)[0])
                                outer = layout.bounds(ppu_frame())
                                execute("xdotool", "mousemove", "--window", native, str(picture["x"] - outer["x"] + 20), str(picture["y"] - outer["y"] + 20))
                                wait(lambda: any("0x" in node.name for node in layout.descendants(ppu_frame()) if node.getRoleName() == "label"))
                                ppu_snapshot(native, "ppu-hover")
                                execute("xdotool", "mousemove", "--window", native, "500", "500", "click", "--repeat", "12", "--delay", "50", "5")
                                wait(lambda: any("== PPU flags ==" in node.name and layout.visible_in_frame(node, ppu_frame()) for node in layout.descendants(ppu_frame()) if node.getRoleName() == "label"))
                                ppu_snapshot(native, "ppu-panels")
                                action = ppu_frame().queryAction()
                                close_index = next(index for index in range(action.nActions) if action.getName(index) == "window.close")
                                if not action.doAction(close_index):
                                    raise RuntimeError("PPU native close request was rejected")
                                wait(lambda: find_ppu() is None)
                                native = open_ppu()
                                ppu_snapshot(native, "ppu-reopened")
                            execute("xdotool", "windowfocus", "--sync", window)
                        if settings_review:
                            execute("xdotool", "key", "ctrl+comma")

                            def find_settings(process=process):
                                result = subprocess.run(["xdotool", "search", "--all", "--onlyvisible", "--pid", str(process.pid), "--name", "^nerust Settings$"], capture_output=True, text=True, check=False)
                                return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout.strip() else None

                            settings_window = wait(find_settings)
                            execute("xdotool", "windowfocus", "--sync", settings_window)
                            execute("xdotool", "windowsize", "--sync", settings_window, "900", "560")

                            def settings_frame(process=process, settings_window=settings_window):
                                return layout.window(process.pid, execute("xdotool", "getwindowname", settings_window))

                            def settings_snapshot(page, expected, process=process, settings_window=settings_window):
                                image = output / f"settings-{page.lower()}.png"
                                def inspect(process=process, settings_window=settings_window):
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
                                    footer = [node for node in visible if node.getRoleName() == "push button" and node.name in {"Cancel", "OK"}]
                                    if {node.name for node in footer} != {"Cancel", "OK"}:
                                        raise RuntimeError("Settings footer is missing controls")
                                    outer = layout.bounds(frame)
                                    for node in footer:
                                        rectangle = layout.bounds(node)
                                        if rectangle["y"] + rectangle["height"] > outer["y"] + outer["height"] - 12:
                                            raise RuntimeError("Settings footer does not preserve its bottom margin")
                                    if not all(layout.normalized(word) in layout.normalized(labels) for word in expected):
                                        return None
                                    if page == "Notifications":
                                        sections = [layout.bounds(node) for node in visible if node.name in {"Information", "Warning"}]
                                        if len(sections) != 2 or sections[0] == sections[1]:
                                            raise RuntimeError("Notifications sections overlap or are missing")
                                    width, height = map(int, execute("identify", "-format", "%w %h", str(image)).split())
                                    if (width, height) != (900, 560):
                                        raise RuntimeError(f"Settings size drifted: {width}x{height}")
                                    return {"state": f"settings-{page.lower()}", "image": image.name, "sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "width": width, "height": height, "ocr": text, "layout": tree_file.name, "layout_sha256": hashlib.sha256(tree_file.read_bytes()).hexdigest()}
                                records.append(wait(inspect))
                                print(f"VIEW CAPTURE settings-{page.lower()}", flush=True)

                            def click_page(page):
                                def locate():
                                    frame = settings_frame()
                                    return frame is not None and layout.select_page(frame, page)
                                wait(locate)

                            def click_setting(command, row_label=None):
                                def locate():
                                    frame = settings_frame()
                                    if frame is None:
                                        return None
                                    root = frame
                                    if row_label is not None:
                                        root = next((node for node in layout.descendants(frame) if node.getRoleName() == "label" and node.name == row_label and layout.visible_in_frame(node, frame)), None)
                                        if root is None:
                                            return None
                                        while root != frame and not any(node.getRoleName() == "push button" for node in layout.descendants(root)):
                                            root = root.parent
                                    for node in layout.descendants(root):
                                        if node.getRoleName() != "push button" or node.name != command:
                                            continue
                                        action = node.queryAction()
                                        for index in range(action.nActions):
                                            if action.getName(index) in {"click", "press", "activate", "button.activate"}:
                                                return lambda action=action, index=index: action.doAction(index)
                                    return None
                                if wait(locate)() is False:
                                    raise RuntimeError(f"Settings action was rejected: {row_label}/{command}")
                                if command == "Change":
                                    wait(lambda: any("Press a key" in node.name and layout.visible_in_frame(node, settings_frame()) for node in layout.descendants(settings_frame())))

                            def change_setting_choice(row_label, steps=0, last=False, process=process, settings_window=settings_window):
                                def locate(process=process, settings_window=settings_window):
                                    frame = settings_frame()
                                    if frame is None:
                                        return None
                                    for label in layout.descendants(frame):
                                        if label.getRoleName() != "label" or label.name != row_label or not layout.visible_in_frame(label, frame):
                                            continue
                                        root = label.parent
                                        while root is not None and root != frame:
                                            combo = next((node for node in layout.descendants(root) if node.getRoleName() == "combo box" and layout.visible_in_frame(node, frame)), None)
                                            if combo is not None:
                                                rectangle, outer = layout.bounds(combo), layout.bounds(frame)
                                                return lambda rectangle=rectangle, outer=outer, settings_window=settings_window: execute("xdotool", "mousemove", "--window", settings_window,
                                                    str(rectangle["x"] - outer["x"] + rectangle["width"] // 2),
                                                    str(rectangle["y"] - outer["y"] + rectangle["height"] // 2), "click", "1")
                                            root = root.parent
                                    return None
                                if wait(locate)() is False:
                                    raise RuntimeError(f"Settings choice rejected: {row_label}")
                                try:
                                    wait(lambda process=process: any(node.getRoleName() in {"list box", "list"} and layout.bounds(node) is not None and layout.bounds(node)["width"] > 0 for node in layout.descendants(layout.application(process.pid))))
                                except Exception:
                                    app = layout.application(process.pid)
                                    (output / "settings-choice-timeout.atspi.json").write_text(json.dumps(layout.tree(app), indent=2) + "\n")
                                    print("VIEW CHOICE POPUP TIMEOUT " + repr([(node.name, node.getRoleName(), layout.bounds(node)) for node in layout.descendants(app) if node.getRoleName() != "label"]), flush=True)
                                    raise
                                print(f"VIEW CHOICE OPEN {row_label}", flush=True)
                                execute("xdotool", "key", "End" if last else "Home", *(["Down"] * steps), "Return")
                                print(f"VIEW CHOICE SENT {row_label}", flush=True)

                            settings_snapshot("General", ["Language", "Storage", "OK"])
                            for page, expected in [("Video", ["VSync", "Scaling", "OK"]), ("Audio", ["Volume:", "Latency:", "OK"]), ("Notifications", ["Information", "Warning", "OK"])]:
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
                            settings_snapshot("Notifications-bottom", ["Status display", "Overlay", "OK"])
                            click_page("Input")
                            settings_snapshot("Input", ["Shortcuts", "Select", "Start", "Change", "Clear", "OK"])
                            if settings_bindings:
                                change_setting_choice("Player 1")
                                wait(lambda: not any(node.getRoleName() == "label" and node.name == "A" and layout.visible_in_frame(node, settings_frame()) for node in layout.descendants(settings_frame())))
                                settings_snapshot("Controller-none", ["None", "OK"])
                                change_setting_choice("Player 1", steps=1)
                                settings_snapshot("Controller-restored", ["Select", "Start", "Change", "Clear", "OK"])
                                click_setting("Change", "A")
                                execute("xdotool", "key", "F7")
                                settings_snapshot("Input-captured", ["F7", "OK"])
                                click_setting("Clear", "A")
                                settings_snapshot("Input-cleared", ["Unbound", "OK"])
                            click_page("Shortcuts")
                            settings_snapshot("Shortcuts", ["Pause", "Reset", "Change", "Clear", "OK"])
                            if settings_bindings:
                                click_setting("Change", "Reset")
                                execute("xdotool", "key", "F9")
                                settings_snapshot("Shortcut-captured", ["F9", "OK"])
                                click_setting("Change", "Reset")
                                execute("xdotool", "key", "Escape")
                                settings_snapshot("Shortcut-cancelled", ["F9", "OK"])
                            click_page("System")
                            settings_snapshot("System", ["Filter", "MMC3 IRQ Variant", "OK"])
                            if settings_bindings:
                                change_setting_choice("Filter", last=True)
                                click_setting("OK")
                                wait(lambda: find_settings() is None)
                                execute("xdotool", "windowfocus", "--sync", window)
                                execute("xdotool", "key", "ctrl+comma")
                                settings_window = wait(find_settings)
                                execute("xdotool", "windowfocus", "--sync", settings_window)
                                execute("xdotool", "windowsize", "--sync", settings_window, "900", "560")
                                click_page("Input")
                                click_page("Shortcuts")
                                settings_snapshot("Shortcut-persisted", ["F9", "OK"])
                                click_setting("Cancel")
                                wait(lambda: find_settings() is None)
                            execute("xdotool", "windowfocus", "--sync", window)
                    execute("xdotool", "key", "ctrl+q")
                    exit_code = process.wait(timeout=15)
                    if exit_code != 0:
                        raise RuntimeError(f"View review shutdown failed: {exit_code}")
                    shutdown_exit_codes[mode] = exit_code
                finally:
                    if process.poll() is None:
                        if accessibility:
                            try:
                                app = layout.application(process.pid)
                                if app is not None:
                                    (output / f"{mode}-failure.atspi.json").write_text(json.dumps(layout.tree(app), indent=2) + "\n")
                            except Exception as diagnostic_error:
                                print(f"VIEW DIAGNOSTIC UNAVAILABLE: {diagnostic_error}", flush=True)
                        process.terminate()
                    process.wait(timeout=15)
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
