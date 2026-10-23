#!/usr/bin/env python3
"""Fail when an Android test exists but is not executed.

Two inventory gaps this guards:
- e2e: MainActivityE2eTest methods must each be selected in e2e.sh.
  A new test that is never selected passes CI silently.
- jvm: @Test methods in src/test must each appear in the JUnit XML
  produced by :app:testDebugUnitTest. A class the build does not
  compile into the test task passes CI silently.

Only inventory is checked here; results are asserted by the runners.
"""
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from bridge_schema import require


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
    """{(class, method)} from this run's JUnit XML."""
    xmls = sorted(results.glob("TEST-*.xml"))
    require(xmls, f"no JUnit XML in {results}")
    found = {}
    for xml in xmls:
        for case in ET.parse(xml).getroot().iter("testcase"):
            name = case.get("name")
            classname = (case.get("classname") or "").split(".")[-1]
            require(name and classname, f"unnamed case in {xml.name}")
            found.setdefault(classname, set()).add(name)
    return found


def check_jvm(results_dir):
    """Every src/test method ran in this JUnit result set, and vice versa."""
    source = jvm_source_tests()
    executed = jvm_executed_tests(Path(results_dir))
    missing = {
        cls: sorted(methods - executed.get(cls, set()))
        for cls, methods in source.items()
        if methods - executed.get(cls, set())
    }
    extra = {
        cls: sorted(methods - source.get(cls, set()))
        for cls, methods in executed.items()
        if cls not in source or methods - source.get(cls, set())
    }
    require(not missing, f"src/test methods missing from JUnit results: {missing}")
    require(not extra, f"JUnit results without src/test methods (stale dir?): {extra}")
    total = sum(len(methods) for methods in source.values())
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
