import re

import pyatspi


def normalized(text):
    return re.sub(r"[^a-z0-9]", "", text.casefold())


def descendants(accessible):
    yield accessible
    for index in range(accessible.childCount):
        child = accessible.getChildAtIndex(index)
        if child is not None:
            yield from descendants(child)


def application(process_id):
    desktop = pyatspi.Registry.getDesktop(0)
    for index in range(desktop.childCount):
        candidate = desktop.getChildAtIndex(index)
        if candidate.get_process_id() == process_id:
            return candidate
    return None


def window(process_id, title):
    app = application(process_id)
    if app is None:
        return None
    for accessible in descendants(app):
        if accessible.getRoleName() in {"frame", "dialog", "window"} and normalized(accessible.name) == normalized(title):
            return accessible
    return None


def bounds(accessible):
    # Probe AT-SPI extents; missing method or stale object both mean
    # "no geometry available" for this poll round.
    try:
        rectangle = accessible.queryComponent().getExtents(pyatspi.WINDOW_COORDS)
        return {"x": rectangle.x, "y": rectangle.y, "width": rectangle.width, "height": rectangle.height}
    except NotImplementedError:
        return None
    except RuntimeError:
        return None


def in_frame(accessible, frame):
    rectangle, origin = bounds(accessible), bounds(frame)
    return rectangle is not None and origin is not None and rectangle["width"] > 0 and rectangle["height"] > 0 and rectangle["x"] < origin["x"] + origin["width"] and rectangle["y"] < origin["y"] + origin["height"] and rectangle["x"] + rectangle["width"] > origin["x"] and rectangle["y"] + rectangle["height"] > origin["y"]


def visible_in_frame(accessible, frame):
    if not in_frame(accessible, frame):
        return False
    parent = accessible.parent
    while parent is not None and parent != frame:
        rectangle = bounds(parent)
        if rectangle is not None and (rectangle["width"] <= 0 or rectangle["height"] <= 0):
            return False
        if parent.getRoleName() in {"scroll pane", "viewport"} and not in_frame(accessible, parent):
            return False
        parent = parent.parent
    return True


def _action_names(accessible):
    # List AT-SPI actions; unimplemented or stale objects expose none.
    try:
        action = accessible.queryAction()
        return [action.getName(index) for index in range(action.nActions)]
    except NotImplementedError:
        return []
    except RuntimeError:
        return []


def _text_payload(accessible, node):
    # Attach plain text where supported; missing APIs leave node bare.
    try:
        text = accessible.queryText()
    except NotImplementedError:
        return
    except RuntimeError:
        return
    node["text"] = text.getText(0, text.characterCount)
    if not text.characterCount:
        return
    try:
        attributes, start, end = text.getAttributeRun(0, True)
    except NotImplementedError:
        return
    except RuntimeError:
        return
    node["text_attribute_run"] = {"attributes": attributes, "start": start, "end": end}


def tree(accessible, parent_bounds=None):
    node = {"name": accessible.name, "role": accessible.getRoleName(), "showing": accessible.getState().contains(pyatspi.STATE_SHOWING), "bounds": bounds(accessible)}
    if parent_bounds is None:
        node["coordinate_system"] = "window-relative logical pixels"
    node["states"] = [str(state) for state in accessible.getState().getStates()]
    rectangle = node["bounds"]
    if rectangle is not None and parent_bounds is not None:
        node["parent_allocation_insets"] = {"left": rectangle["x"] - parent_bounds["x"], "top": rectangle["y"] - parent_bounds["y"], "right": parent_bounds["x"] + parent_bounds["width"] - rectangle["x"] - rectangle["width"], "bottom": parent_bounds["y"] + parent_bounds["height"] - rectangle["y"] - rectangle["height"]}
    node["actions"] = _action_names(accessible)
    _text_payload(accessible, node)
    node["children"] = [tree(accessible.getChildAtIndex(index), rectangle) for index in range(accessible.childCount) if accessible.getChildAtIndex(index) is not None]
    for previous, current in zip(node["children"], node["children"][1:]):
        before, after = previous["bounds"], current["bounds"]
        if before is not None and after is not None:
            current["previous_sibling_distance"] = {"horizontal": after["x"] - before["x"] - before["width"], "vertical": after["y"] - before["y"] - before["height"]}
    return node


def _try_select_child(target):
    # Attempt one AT-SPI selection step; missing/stale APIs yield None
    # so the caller can walk up to the parent container instead.
    try:
        selection = target.parent.querySelection()
        return bool(selection.selectChild(target.getIndexInParent()))
    except NotImplementedError:
        return None
    except RuntimeError:
        return None


def select_page(frame, page):
    for accessible in descendants(frame):
        if normalized(accessible.name) != normalized(page) or not in_frame(accessible, frame):
            continue
        target = accessible
        for _ in range(3):
            if _try_select_child(target):
                return True
            target = target.parent
            if target is None or target == frame:
                break
    return False
