#!/usr/bin/env python3
"""Flavor dependency gate: wakui-tao/wakui-gtk stay framework-only.

Checks:
1. `[dependencies]` (plus target-specific and `[dev-dependencies]`)
   of `wakui-tao`/`wakui-gtk` name zero `nerust_*` crates.
2. `cargo tree --edges normal --depth 1` agrees (no direct normal edge
   to any `nerust_*` crate).
3. All `.rs` files in both flavors (production AND tests) name zero
   `nerust_*` crates. String literals and comments are ignored, so
   native window role names like "nerust_debug" do not count.
4. `cargo tree -p wakui --edges normal` contains no `nerust_*`.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
FLAVORS = {
    "wakui-tao": ROOT / "gui/wakui-tao",
    "wakui-gtk": ROOT / "gui/wakui-gtk",
}


def strip_code(line: str) -> str:
    """Remove string literals and line comments for structural scanning."""
    code = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
    code = re.sub(r"//.*$", "", code)
    return code


def toml_nerust_deps(manifest: Path) -> set[str]:
    deps: set[str] = set()
    section: str | None = None
    for line in manifest.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            # [dependencies], [dev-dependencies], [target.*.dependencies]
            section = stripped
            continue
        if section is not None and section.endswith("dependencies]"):
            match = re.match(r"([a-z0-9_-]+)[\s.=]", stripped)
            if match and match.group(1).startswith("nerust_"):
                deps.add(match.group(1))
    return deps


def rs_uses(crate_dir: Path) -> dict[str, list[str]]:
    hits: dict[str, list[str]] = {}
    for path in sorted((crate_dir / "src").rglob("*.rs")):
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
        match = re.search(r"(nerust_[a-z0-9_]+) v", line)
        if match and match.group(1) != package:
            found.append(match.group(1))
    return sorted(set(found))


def main() -> int:
    failures: list[str] = []

    for package, crate_dir in FLAVORS.items():
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

    wakui_closure = cargo_closure_has_nerust("wakui")
    if wakui_closure:
        failures.append(f"wakui normal closure contains nerust_*: {wakui_closure}")

    if failures:
        print("flavor policy gate FAILED:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("flavor policy gate OK: wakui-tao/wakui-gtk are nerust_*-free; wakui is repo-free")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
