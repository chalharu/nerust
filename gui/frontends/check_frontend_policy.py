#!/usr/bin/env python3
"""Phase 4 dependency gate: tao/gtk stay thin binders over the framework.

Checks (mechanical verification for design section 8):
1. `[dependencies]` of `nerust_tao`/`nerust_gtk` name only
   `nerust_frontend_host` and `wakui` among `nerust_*` crates.
2. `cargo tree --edges normal --depth 1` agrees (no direct normal edge
   to view-defs, ViewModel, kernel, or settings crates).
3. Production (non-`#[cfg(test)]`) code in both frontends names only
   `nerust_frontend_host` and `wakui` (`nerust_render_*` peer assets are
   reported separately, never as violations).
4. `cargo tree -p wakui --edges normal` contains no `nerust_*`.

Test-only files (`tests.rs`, `test_*.rs`, `*_tests.rs`, `tests/`,
`testkit.rs`) and `#[cfg(test)]` items are excluded from the `use` scan.
Host (`nerust_frontend_host`) keeps its ViewModel wiring: goal 3
(host sheds its intra-repo deps via the ApplicationHost-to-view-defs
move) is tracked separately and is NOT part of this gate.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FRONTENDS = {
    "nerust_tao": ROOT / "gui/frontends/tao",
    "nerust_gtk": ROOT / "gui/frontends/gtk",
}
ALLOWED_PROD = {"nerust_frontend_host", "wakui"}

TEST_BASENAMES = {"testkit.rs"}
TEST_PREFIXES = ("test_",)
TEST_SUFFIXES = ("tests.rs", "_tests.rs")


def is_test_file(path: Path) -> bool:
    name = path.name
    return (
        name in TEST_BASENAMES
        or name.startswith(TEST_PREFIXES)
        or name.endswith(TEST_SUFFIXES)
        or "tests" in path.parts
    )


def strip_code(line: str) -> str:
    """Remove string literals and line comments for structural scanning."""
    code = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
    code = re.sub(r"//.*$", "", code)
    return code


def strip_test_items(lines: list[str]) -> list[str]:
    """Remove `#[cfg(test)]`-attributed items via brace/semicolon skipping."""
    out: list[str] = []
    i = 0
    while i < len(lines):
        if lines[i].strip() == "#[cfg(test)]":
            i += 1
            while i < len(lines) and (
                not lines[i].strip()
                or lines[i].strip().startswith(("///", "//", "#["))
            ):
                i += 1
            if i >= len(lines):
                break
            head = lines[i].strip()
            if re.match(
                r"^(pub(\(.*\))?\s+)?(mod|fn|impl|struct|enum|trait|union)\b", head
            ):
                depth = 0
                opened = False
                while i < len(lines):
                    code = strip_code(lines[i])
                    if "{" in code:
                        opened = True
                    depth += code.count("{") - code.count("}")
                    i += 1
                    if opened and depth <= 0:
                        break
                    if not opened and ";" in code:
                        break
                continue
            if re.match(r"^(pub(\(.*\))?\s+)?(use|const|static|type)\b", head):
                while i < len(lines) and ";" not in lines[i]:
                    i += 1
                i += 1
                continue
            i += 1
            continue
        out.append(lines[i])
        i += 1
    return out


def prod_uses(crate: str, src: Path) -> dict[str, list[str]]:
    hits: dict[str, list[str]] = {}
    for path in sorted(src.rglob("*.rs")):
        if is_test_file(path):
            continue
        lines = strip_test_items(path.read_text().splitlines())
        for num, line in enumerate(lines, 1):
            for match in re.finditer(r"nerust_[a-z0-9_]+", strip_code(line)):
                name = match.group(0)
                if name in ALLOWED_PROD or name.startswith("nerust_render_"):
                    continue
                hits.setdefault(name, []).append(f"{path.relative_to(ROOT)}:{num}")
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
        match = re.match(r"([a-z0-9_]+) v", line.strip())
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
        match = re.search(r"(nerust_[a-z0-9_]+) v", line)
        if match and match.group(1) != package:
            found.append(match.group(1))
    return sorted(set(found))


def toml_normal_deps(manifest: Path) -> set[str]:
    deps: set[str] = set()
    section: str | None = None
    for line in manifest.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            section = stripped
            continue
        if section == "[dependencies]":
            match = re.match(r"(nerust_[a-z0-9_]+)[\s.=]", stripped)
            if match:
                deps.add(match.group(1))
    return deps


def main() -> int:
    failures: list[str] = []

    for package, crate_dir in FRONTENDS.items():
        direct = toml_normal_deps(crate_dir / "Cargo.toml")
        bad = direct - ALLOWED_PROD
        if bad:
            failures.append(f"{package} Cargo.toml [dependencies]: {sorted(bad)}")

        tree_direct = cargo_direct_deps(package)
        bad = tree_direct - ALLOWED_PROD
        if bad:
            failures.append(f"{package} cargo tree direct normal deps: {sorted(bad)}")

        for name, sites in prod_uses(package, crate_dir / "src").items():
            shown = ", ".join(sites[:5])
            failures.append(f"{package} prod code names {name}: {shown}")

    wakui_closure = cargo_closure_has_nerust("wakui")
    if wakui_closure:
        failures.append(f"wakui normal closure contains nerust_*: {wakui_closure}")

    if failures:
        print("frontend policy gate FAILED:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("frontend policy gate OK: tao/gtk depend only on host + wakui; wakui is repo-free")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
