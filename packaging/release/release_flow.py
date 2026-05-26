#!/usr/bin/env python3

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from datetime import date
from pathlib import Path
from typing import Any


WORKSPACE_ROOT = Path(__file__).resolve().parents[2]
CARGO_TOML = WORKSPACE_ROOT / "Cargo.toml"
CHANGELOG = WORKSPACE_ROOT / "CHANGELOG.md"
PATCH_EXACT_PATHS = {
    "CHANGELOG.md",
    "Cargo.lock",
    "README.md",
    "deny.toml",
    "renovate.json",
}
PATCH_PREFIXES = (".github/", "packaging/")
DEPENDENCY_TABLE_KEYS = {"dependencies", "dev-dependencies", "build-dependencies"}
DEPENDENCY_VALUE_KEYS = {"branch", "registry", "rev", "tag", "version"}
REPOSITORY_URL = "https://github.com/chalharu/nerust"


@dataclass(frozen=True)
class Version:
    major: int
    minor: int
    patch: int

    @classmethod
    def parse(cls, text: str) -> "Version":
        parts = text.strip().split(".")
        if len(parts) != 3:
            raise ValueError(f"invalid version: {text}")
        return cls(*(int(part) for part in parts))

    def tag_name(self) -> str:
        return f"v{self}"

    def bump_patch(self) -> "Version":
        return Version(self.major, self.minor, self.patch + 1)

    def bump_minor(self) -> "Version":
        return Version(self.major, self.minor + 1, 0)

    def __str__(self) -> str:
        return f"{self.major}.{self.minor}.{self.patch}"


def git(*args: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=WORKSPACE_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return completed.stdout.strip()


def git_ref_is_ancestor(ancestor: str, descendant: str) -> bool:
    completed = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=WORKSPACE_ROOT,
        check=False,
    )
    return completed.returncode == 0


def read_workspace_version() -> Version:
    cargo = tomllib.loads(CARGO_TOML.read_text())
    return Version.parse(cargo["workspace"]["package"]["version"])


def latest_tag_for_major(major: int) -> str | None:
    tags = git("tag", "--list", f"v{major}.*", "--sort=-version:refname").splitlines()
    return tags[0] if tags else None


def changed_files(base_ref: str) -> list[str]:
    changed = git("diff", "--name-only", f"{base_ref}..HEAD")
    return [line for line in changed.splitlines() if line]


def git_show_text(ref: str, path: str) -> str:
    return git("show", f"{ref}:{path}")


def dependency_spec_patch_safe(old_value: Any, new_value: Any) -> bool:
    if type(old_value) is not type(new_value):
        return False
    if isinstance(old_value, str):
        return True
    if isinstance(old_value, dict):
        if set(old_value) != set(new_value):
            return False
        for key in old_value:
            if old_value[key] != new_value[key] and key not in DEPENDENCY_VALUE_KEYS:
                return False
        return True
    if isinstance(old_value, list):
        return old_value == new_value
    return old_value == new_value


def dependency_table_patch_safe(
    old_table: dict[str, Any], new_table: dict[str, Any]
) -> bool:
    if set(old_table) != set(new_table):
        return False
    return all(
        dependency_spec_patch_safe(old_table[name], new_table[name])
        for name in old_table
    )


def cargo_toml_patch_safe(old_value: Any, new_value: Any) -> bool:
    if type(old_value) is not type(new_value):
        return False
    if isinstance(old_value, dict):
        all_keys = set(old_value) | set(new_value)
        for key in all_keys:
            if key not in old_value or key not in new_value:
                return False
            if key in DEPENDENCY_TABLE_KEYS:
                if not isinstance(old_value[key], dict) or not isinstance(
                    new_value[key], dict
                ):
                    return False
                if not dependency_table_patch_safe(old_value[key], new_value[key]):
                    return False
                continue
            if not cargo_toml_patch_safe(old_value[key], new_value[key]):
                return False
        return True
    return old_value == new_value


def cargo_file_patch_safe(path: str, base_ref: str) -> bool:
    try:
        old_text = git_show_text(base_ref, path)
    except subprocess.CalledProcessError:
        return False
    try:
        new_text = (WORKSPACE_ROOT / path).read_text()
    except FileNotFoundError:
        return False
    return cargo_toml_patch_safe(tomllib.loads(old_text), tomllib.loads(new_text))


def patch_only_since(base_ref: str) -> bool:
    files = changed_files(base_ref)
    for path in files:
        if path in PATCH_EXACT_PATHS or any(
            path.startswith(prefix) for prefix in PATCH_PREFIXES
        ):
            continue
        if path.endswith("Cargo.toml") and cargo_file_patch_safe(path, base_ref):
            continue
        return False
    return True


def next_version() -> tuple[Version, str, str]:
    declared = read_workspace_version()
    base_tag = latest_tag_for_major(declared.major)
    if base_tag is None:
        return declared, "", "initial"

    if not git_ref_is_ancestor(base_tag, "HEAD"):
        raise RuntimeError(
            f"{base_tag} is not reachable from HEAD; merge the previous release sync before preparing the next candidate"
        )

    if not changed_files(base_tag):
        raise RuntimeError(f"no releaseable changes found since {base_tag}")

    base_version = Version.parse(base_tag.removeprefix("v"))
    if patch_only_since(base_tag):
        return base_version.bump_patch(), base_tag, "patch"
    return base_version.bump_minor(), base_tag, "minor"


def workspace_version_dependency_names() -> set[str]:
    cargo = tomllib.loads(CARGO_TOML.read_text())
    names: set[str] = set()
    for name, spec in cargo["workspace"]["dependencies"].items():
        if not isinstance(spec, dict) or "path" not in spec or "version" not in spec:
            continue
        manifest = WORKSPACE_ROOT / spec["path"] / "Cargo.toml"
        if not manifest.exists():
            continue
        package = tomllib.loads(manifest.read_text()).get("package", {})
        package_version = package.get("version")
        if (
            isinstance(package_version, dict)
            and package_version.get("workspace") is True
        ):
            names.add(name)
    return names


def replace_workspace_version(target_version: Version) -> None:
    cargo_lines = CARGO_TOML.read_text().splitlines()
    in_workspace_package = False
    in_workspace_dependencies = False
    version_synced_dependencies = workspace_version_dependency_names()
    updated_lines: list[str] = []

    for line in cargo_lines:
        stripped = line.strip()
        if stripped == "[workspace.package]":
            in_workspace_package = True
            in_workspace_dependencies = False
        elif stripped == "[workspace.dependencies]":
            in_workspace_package = False
            in_workspace_dependencies = True
        elif stripped.startswith("["):
            in_workspace_package = False
            in_workspace_dependencies = False

        if in_workspace_package and stripped.startswith("version = "):
            updated_lines.append(f'version = "{target_version}"')
        elif in_workspace_dependencies:
            match = re.match(
                r'^(\s*([\w.-]+)\s*=\s*\{.*?\bversion\s*=\s*")=[^"]+(".*\}\s*)$',
                line,
            )
            if match and match.group(2) in version_synced_dependencies:
                updated_lines.append(
                    f"{match.group(1)}={target_version}{match.group(3)}"
                )
            else:
                updated_lines.append(line)
        else:
            updated_lines.append(line)

    CARGO_TOML.write_text("\n".join(updated_lines) + "\n")


def build_release_link(version: Version, base_tag: str) -> str:
    if base_tag:
        return f"{REPOSITORY_URL}/compare/{base_tag}...{version.tag_name()}"
    return f"{REPOSITORY_URL}/releases/tag/{version.tag_name()}"


def ensure_changelog(version: Version, base_tag: str) -> None:
    original = CHANGELOG.read_text()
    today = date.today().isoformat()
    heading_pattern = re.compile(
        rf"^## \[{re.escape(str(version))}\] - .*$", re.MULTILINE
    )
    text = original

    if not heading_pattern.search(text):
        marker = "## [Unreleased]\n"
        if marker not in text:
            raise RuntimeError("CHANGELOG.md is missing the Unreleased section")
        insertion = (
            f"## [{version}] - {today}\n\n"
            "### Changed\n\n"
            "- TODO: summarize the release changes.\n\n"
        )
        text = text.replace(marker, marker + "\n" + insertion, 1)

    marker = "<!-- next-url -->"
    if marker not in text:
        raise RuntimeError("CHANGELOG.md is missing the next-url marker")

    before_links, after_links = text.split(marker, 1)
    existing_links: list[tuple[str, str]] = []
    for line in after_links.strip().splitlines():
        match = re.match(r"^\[(.+?)\]:\s*(.+)$", line.strip())
        if match:
            existing_links.append((match.group(1), match.group(2)))

    ordered_links: list[tuple[str, str]] = [
        ("Unreleased", f"{REPOSITORY_URL}/compare/{version.tag_name()}...HEAD"),
        (str(version), build_release_link(version, base_tag)),
    ]
    seen = {name for name, _ in ordered_links}
    for name, url in existing_links:
        if name not in seen:
            ordered_links.append((name, url))
            seen.add(name)

    link_block = "\n".join(f"[{name}]: {url}" for name, url in ordered_links)
    CHANGELOG.write_text(f"{before_links}{marker}\n{link_block}\n")


def extract_release_notes(version: Version) -> str:
    text = CHANGELOG.read_text()
    lines = text.splitlines()
    start_prefix = f"## [{version}] - "
    start_index = -1
    for index, line in enumerate(lines):
        if line.startswith(start_prefix):
            start_index = index + 1
            break
    if start_index == -1:
        raise RuntimeError(f"CHANGELOG.md is missing the {version} section")

    end_index = len(lines)
    for index in range(start_index, len(lines)):
        if lines[index].startswith("## [") or lines[index].startswith(
            "<!-- next-url -->"
        ):
            end_index = index
            break

    notes = "\n".join(lines[start_index:end_index]).strip()
    if not notes:
        raise RuntimeError(f"CHANGELOG.md has no notes for {version}")
    return notes + "\n"


def print_metadata(metadata: dict[str, str]) -> None:
    for key, value in metadata.items():
        print(f"{key}={value}")


def command_workspace_metadata(_: argparse.Namespace) -> int:
    version = read_workspace_version()
    print_metadata({"version": str(version), "tag_name": version.tag_name()})
    return 0


def command_next_version(_: argparse.Namespace) -> int:
    version, base_tag, bump_kind = next_version()
    print_metadata(
        {
            "version": str(version),
            "tag_name": version.tag_name(),
            "base_tag": base_tag,
            "bump_kind": bump_kind,
        }
    )
    return 0


def command_prepare_candidate(_: argparse.Namespace) -> int:
    version, base_tag, bump_kind = next_version()
    replace_workspace_version(version)
    ensure_changelog(version, base_tag)
    print_metadata(
        {
            "version": str(version),
            "tag_name": version.tag_name(),
            "base_tag": base_tag,
            "bump_kind": bump_kind,
        }
    )
    return 0


def command_release_notes(args: argparse.Namespace) -> int:
    notes = extract_release_notes(Version.parse(args.version))
    if args.output:
        Path(args.output).write_text(notes)
    else:
        sys.stdout.write(notes)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    workspace_metadata = subparsers.add_parser("workspace-metadata")
    workspace_metadata.set_defaults(func=command_workspace_metadata)

    next_version_parser = subparsers.add_parser("next-version")
    next_version_parser.set_defaults(func=command_next_version)

    prepare_candidate = subparsers.add_parser("prepare-candidate")
    prepare_candidate.set_defaults(func=command_prepare_candidate)

    release_notes = subparsers.add_parser("release-notes")
    release_notes.add_argument("version")
    release_notes.add_argument("--output")
    release_notes.set_defaults(func=command_release_notes)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
