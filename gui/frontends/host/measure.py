"""Compare physical Rust source lines, including comments, blanks and tests."""

import argparse
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[3]
BASELINE = "77a6d7b1a2260f10d1fa5ac0d59cd5b386cef54c"


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT).decode()


def sources_at(base, directory):
    paths = git("ls-tree", "-r", "--name-only", base, directory).splitlines()
    return {
        path: git("show", f"{base}:{path}")
        for path in paths
        if path.endswith(".rs")
    }


def current_sources(directory):
    return {
        path.relative_to(ROOT).as_posix(): path.read_text()
        for path in (ROOT / directory).rglob("*.rs")
    }


def lines(sources):
    return sum(len(source.splitlines()) for source in sources.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default=BASELINE)
    args = parser.parse_args()
    print(f"Base: `{git('rev-parse', args.base).strip()}`\n")
    print("| Scope | Before | After | Delta | Reduction |")
    print("| --- | ---: | ---: | ---: | ---: |")
    totals = [0, 0]
    for name in ["tao", "gtk", "host"]:
        directory = f"gui/frontends/{name}/src"
        before = sources_at(args.base, directory)
        after = current_sources(directory)
        a, b = lines(before), lines(after)
        totals[0] += a
        totals[1] += b
        reduction = f"{(a - b) * 100 / a:.1f}%" if a else "new"
        print(f"| {name} | {a} | {b} | {b - a:+} | {reduction} |")
    a, b = totals
    print(f"| Combined | {a} | {b} | {b - a:+} | {(a - b) * 100 / a:.1f}% |")


if __name__ == "__main__":
    main()