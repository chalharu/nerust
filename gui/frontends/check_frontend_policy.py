#!/usr/bin/env python3
"""Frontend dependency gate.

Panechord manifests, dependency closures and Rust sources stay framework-only.
Native backend selection belongs to Panechord, not the desktop adapter.
Inner layers depend on neither the UI framework nor the removed Wakui stack.
"""

import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FRAMEWORKS = {"panechord": ROOT / "panechord"} | {
    f"panechord-{name}": ROOT / "panechord" / name
    for name in ("runtime", "markup", "macros", "native", "gtk", "tao", "macos", "demo", "settings", "library")
}


def strip_code(line: str) -> str:
    """Remove string literals and line comments for structural scanning."""
    code = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
    code = re.sub(r"//.*$", "", code)
    return code


def toml_dependency_names(manifest: Path) -> set[str]:
    data = tomllib.loads(manifest.read_text())
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["dependencies"]
    deps: set[str] = set()
    for table in (data, *data.get("target", {}).values()):
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, value in table.get(section, {}).items():
                if isinstance(value, dict) and value.get("workspace"):
                    value = workspace[name]
                deps.add(value.get("package", name) if isinstance(value, dict) else name)
    return deps


def toml_nerust_deps(manifest: Path) -> set[str]:
    return {name for name in toml_dependency_names(manifest) if name == "nerust" or name.startswith("nerust_")}


def rs_uses(crate_dir: Path) -> dict[str, list[str]]:
    hits: dict[str, list[str]] = {}
    sources = (path for directory in ("src", "tests", "examples") for path in (crate_dir / directory).rglob("*.rs"))
    for path in sorted(sources):
        for num, line in enumerate(path.read_text().splitlines(), 1):
            for match in re.finditer(r"nerust_[a-z0-9_]+", strip_code(line)):
                hits.setdefault(match.group(0), []).append(
                    f"{path.relative_to(ROOT)}:{num}"
                )
    return hits


def cargo_direct_deps(package: str) -> set[str]:
    out = subprocess.run(
        ["cargo", "tree", "-p", package, "--edges", "normal",
         "--depth", "1", "--prefix", "none", "--locked"],
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    if out.returncode != 0:
        print(out.stderr[-2000:])
        raise SystemExit(f"cargo tree failed for {package}")
    deps = set()
    for line in out.stdout.splitlines():
        match = re.match(r"([a-z0-9_-]+) v", line.strip())
        if match and match.group(1).startswith("nerust_"):
            deps.add(match.group(1))
    return deps - {package}


def cargo_closure_has_nerust(package: str) -> list[str]:
    out = subprocess.run(
        ["cargo", "tree", "-p", package, "--edges", "normal",
         "--prefix", "none", "--locked"],
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    if out.returncode != 0:
        print(out.stderr[-2000:])
        raise SystemExit(f"cargo tree failed for {package}")
    found = []
    for line in out.stdout.splitlines():
        match = re.search(r"(nerust(?:_[a-z0-9_]+)?) v", line)
        if match and match.group(1) != package:
            found.append(match.group(1))
    return sorted(set(found))


def desktop_backend_violations(crate_dir: Path) -> list[str]:
    failures: list[str] = []
    forbidden = {
        "gtk4", "gtk", "gdk4", "tao", "muda", "iced", "objc", "objc2",
        "panechord-gtk", "panechord-tao", "panechord-macos", "panechord-native",
    }
    dependencies = toml_dependency_names(crate_dir / "Cargo.toml") & forbidden
    if dependencies:
        failures.append(f"desktop native backend dependencies: {sorted(dependencies)}")
    patterns = (
        r"panechord::(?:tao|gtk|macos)\b",
        r"\b(?:panechord_tao|panechord_gtk|panechord_macos|gtk|tao|iced|muda|objc2)::",
        r'feature\s*=\s*"(?:tao|gtk|macos)"',
        r'\btarget_os\s*=',
    )
    for directory in ("src", "examples", "tests"):
        for path in sorted((crate_dir / directory).rglob("*.rs")):
            for number, line in enumerate(path.read_text().splitlines(), 1):
                code = re.sub(r"//.*$", "", line)
                if any(re.search(pattern, code) for pattern in patterns):
                    failures.append(f"desktop backend-specific code: {path.relative_to(ROOT)}:{number}")
    return failures


def main() -> int:
    failures: list[str] = []
    failures.extend(desktop_backend_violations(ROOT / "gui/frontends/desktop"))

    for package, crate_dir in FRAMEWORKS.items():
        bad = toml_nerust_deps(crate_dir / "Cargo.toml")
        if bad:
            failures.append(f"{package} Cargo.toml dependencies: {sorted(bad)}")

        tree_direct = cargo_direct_deps(package)
        if tree_direct:
            failures.append(
                f"{package} cargo tree direct normal deps: {sorted(tree_direct)}"
            )

        for name, sites in rs_uses(crate_dir).items():
            shown = ", ".join(sites[:5])
            failures.append(f"{package} code names {name}: {shown}")

    for package in FRAMEWORKS:
        bad = cargo_closure_has_nerust(package)
        if bad:
            failures.append(f"{package} normal closure contains nerust: {bad}")
    for domain in ("gui/runtime", "gui/shell", "gui/viewmodel", "gui/application", "settings/core", "traits/input"):
        bad = sorted(name for name in toml_dependency_names(ROOT / domain / "Cargo.toml") if name.startswith(("panechord", "wakui")))
        if bad:
            failures.append(f"inner domain {domain} depends on a UI framework: {bad}")

    if failures:
        print("flavor policy gate FAILED:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("frontend policy gate OK: Panechord owns native backends; desktop is backend-neutral; inner domains have no Panechord dependency")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
