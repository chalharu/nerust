#!/usr/bin/env python3
"""Fail when an Android test exists but is not executed.

Inventory gaps this guards:
- e2e: MainActivityE2eTest methods must each be selected in e2e.sh.
  A new test that is never selected passes CI silently.
- jvm: @Test methods in src/test must each appear executed in the JUnit XML
  produced by :app:testDebugUnitTest. A class the build does not
  compile into the test task passes CI silently.
- skipped JVM cases count as unexecuted: only reasoned, dated method-level
  allowances pass the gate, and expired allowances fail it.

Only inventory is checked here; results are asserted by the runners.
"""
import datetime
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from bridge_schema import require

#: Intentionally skipped JVM tests: "Class.method" to (reason, review-by date).
#: An expired entry fails the gate; remove it or re-justify.
ALLOWED_JVM_SKIPS = {
    # Example: "SafCleanupTest.flakyOnEmulator": ("needs device", "2026-12-31"),
}


def kotlin_test_methods(path):
    """@Test method names in declaration order."""
    text = path.read_text()
    return re.findall(r"@Test(?:\([^)]*\))?\s+fun\s+(\w+)", text)


def selected_e2e_tests(text):
    """TEST_CLASS selections on live shell lines. Full-line comments never
    run, and trailing comments are cut outside double quotes, so commented
    selections cannot pass the inventory gate (NER-315)."""
    live = []
    for line in text.splitlines():
        stripped = line.lstrip()
        if not stripped or stripped.startswith("#"):
            continue
        kept = []
        in_quotes = False
        for char in stripped:
            if char == '"':
                in_quotes = not in_quotes
            elif char == "#" and not in_quotes:
                break
            kept.append(char)
        live.append("".join(kept))
    invoked = set()
    for chunk in re.findall(r"\$\{TEST_CLASS\}#([A-Za-z0-9_,]+)", "\n".join(live)):
        invoked.update(name for name in chunk.split(",") if name)
    return invoked


def diff_names(defined, invoked):
    """Sorted (missing, extra) lists for the inventory gate."""
    return sorted(defined - invoked), sorted(invoked - defined)


def check_e2e():
    """Every instrumentation test is selected in e2e.sh, and vice versa."""
    e2e = (HERE / "e2e.sh").read_text()
    invoked = selected_e2e_tests(e2e)
    require(invoked, "no instrumented tests selected in e2e.sh")
    test_file = HERE / "app/src/androidTest/kotlin/io/github/chalharu/nerust/MainActivityE2eTest.kt"
    defined = set(kotlin_test_methods(test_file))
    require(defined, "no @Test methods in MainActivityE2eTest")
    missing, extra = diff_names(defined, invoked)
    require(not missing, f"instrumentation tests never selected: {missing}")
    require(not extra, f"selected tests not defined: {extra}")
    print(f"e2e test inventory: OK ({len(defined)} tests selected)")


def jvm_source_tests():
    """{(class, method)} from every src/test Test class."""
    found = {}
    for path in sorted((HERE / "app/src/test").rglob("*Test.kt")):
        methods = kotlin_test_methods(path)
        require(methods, f"no @Test methods in {path.name}")
        require(path.stem not in found, f"duplicate test class: {path.stem}")
        found[path.stem] = set(methods)
    require(found, "no JVM test classes found")
    return found


def jvm_executed_tests(results):
    """(executed, skipped) from this run's JUnit XML. A case with a <skipped/>
    child ran nothing: failures and errors count as executed, skips do not."""
    xmls = sorted(results.glob("TEST-*.xml"))
    require(xmls, f"no JUnit XML in {results}")
    executed, skipped = {}, {}
    for xml in xmls:
        for case in ET.parse(xml).getroot().iter("testcase"):
            name = case.get("name")
            classname = (case.get("classname") or "").split(".")[-1]
            require(name and classname, f"unnamed case in {xml.name}")
            target = skipped if case.find("skipped") is not None else executed
            target.setdefault(classname, set()).add(name)
    return executed, skipped


def unexecuted_jvm_tests(source, executed, skipped, allowances, today):
    """(missing, unallowed) for the gate. Allowances map "Class.method" to
    (reason, review-by date); expired entries and entries without a matching
    test fail the gate instead of passing silently."""
    expired = sorted(key for key, (_, date) in allowances.items() if date < today)
    require(not expired, f"expired skip allowances (re-justify or remove): {expired}")
    source_pairs = {(cls, method) for cls, methods in source.items() for method in methods}
    allowed_pairs = set()
    for key in allowances:
        pair = tuple(key.split(".", 1))
        require(len(pair) == 2, f"skip allowance without Class.method shape: {key}")
        allowed_pairs.add(pair)
    unknown = sorted(f"{cls}.{method}" for cls, method in allowed_pairs - source_pairs)
    require(not unknown, f"skip allowances without a matching test: {unknown}")
    executed_pairs = {(cls, method) for cls, methods in executed.items() for method in methods}
    skipped_pairs = {(cls, method) for cls, methods in skipped.items() for method in methods}
    missing = sorted(source_pairs - executed_pairs - allowed_pairs)
    unallowed = sorted(skipped_pairs - allowed_pairs)
    return missing, unallowed


def check_jvm(results_dir):
    """Every src/test method ran in this JUnit result set, and vice versa.
    Skipped cases are unexecuted unless a live allowance names them."""
    source = jvm_source_tests()
    executed, skipped = jvm_executed_tests(Path(results_dir))
    missing, unallowed = unexecuted_jvm_tests(
        source, executed, skipped, ALLOWED_JVM_SKIPS, datetime.date.today().isoformat()
    )
    require(not missing, f"src/test methods missing from JUnit results: {missing}")
    require(not unallowed, f"skipped JVM tests without allowance: {unallowed}")
    extra = {
        cls: sorted(methods - source.get(cls, set()))
        for cls, methods in executed.items()
        if cls not in source or methods - source.get(cls, set())
    }
    require(not extra, f"JUnit results without src/test methods (stale dir?): {extra}")
    total = sum(len(methods) for methods in executed.values())
    print(f"JVM test inventory: OK ({total} tests executed)")


def main(argv):
    if argv == ["e2e"]:
        check_e2e()
    elif len(argv) == 2 and argv[0] == "jvm":
        check_jvm(argv[1])
    else:
        print("usage: check-test-inventory.py e2e | check-test-inventory.py jvm <results-dir>")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
