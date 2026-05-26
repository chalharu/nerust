#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CARGO_TOML="${WORKSPACE_ROOT}/Cargo.toml"
CHANGELOG="${WORKSPACE_ROOT}/CHANGELOG.md"
REPOSITORY_URL="https://github.com/chalharu/nerust"

git_root() {
    git -C "${WORKSPACE_ROOT}" "$@"
}

print_metadata() {
    while (($# > 0)); do
        printf '%s=%s\n' "$1" "$2"
        shift 2
    done
}

read_workspace_version() {
    awk '
        /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
        /^\[/ { in_workspace_package = 0 }
        in_workspace_package && /^[[:space:]]*version[[:space:]]*=/ {
            line = $0
            sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*"/, "", line)
            sub(/".*$/, "", line)
            print line
            exit
        }
    ' "${CARGO_TOML}"
}

latest_tag_for_major() {
    git_root tag --list "v$1.*" --sort=-version:refname | head -n 1
}

git_ref_is_ancestor() {
    git_root merge-base --is-ancestor "$1" "$2"
}

changed_files() {
    git_root diff --name-only "$1..HEAD"
}

cargo_file_patch_safe() {
    local path="$1"
    local base_ref="$2"

    python3 - "${WORKSPACE_ROOT}" "${path}" "${base_ref}" <<'PY'
import pathlib
import subprocess
import sys
import tomllib
from typing import Any

workspace_root = pathlib.Path(sys.argv[1])
path = sys.argv[2]
base_ref = sys.argv[3]
dependency_table_keys = {"dependencies", "dev-dependencies", "build-dependencies"}
dependency_value_keys = {"branch", "registry", "rev", "tag", "version"}

def dependency_spec_patch_safe(old_value: Any, new_value: Any) -> bool:
    if type(old_value) is not type(new_value):
        return False
    if isinstance(old_value, str):
        return True
    if isinstance(old_value, dict):
        if set(old_value) != set(new_value):
            return False
        for key in old_value:
            if old_value[key] != new_value[key] and key not in dependency_value_keys:
                return False
        return True
    if isinstance(old_value, list):
        return old_value == new_value
    return old_value == new_value

def dependency_table_patch_safe(old_table: dict[str, Any], new_table: dict[str, Any]) -> bool:
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
            if key in dependency_table_keys:
                if not isinstance(old_value[key], dict) or not isinstance(new_value[key], dict):
                    return False
                if not dependency_table_patch_safe(old_value[key], new_value[key]):
                    return False
                continue
            if not cargo_toml_patch_safe(old_value[key], new_value[key]):
                return False
        return True
    return old_value == new_value

try:
    old_text = subprocess.run(
        ["git", "-C", str(workspace_root), "show", f"{base_ref}:{path}"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    new_text = (workspace_root / path).read_text()
except (subprocess.CalledProcessError, FileNotFoundError):
    raise SystemExit(1)

raise SystemExit(0 if cargo_toml_patch_safe(tomllib.loads(old_text), tomllib.loads(new_text)) else 1)
PY
}

patch_only_since() {
    local base_ref="$1"
    local path

    while IFS= read -r path; do
        [[ -z "${path}" ]] && continue
        case "${path}" in
            CHANGELOG.md|Cargo.lock|README.md|deny.toml|renovate.json)
                continue
                ;;
            .github/*|packaging/*)
                continue
                ;;
            *Cargo.toml)
                if cargo_file_patch_safe "${path}" "${base_ref}"; then
                    continue
                fi
                ;;
        esac
        return 1
    done < <(changed_files "${base_ref}")

    return 0
}

replace_workspace_version() {
    local target_version="$1"
    local -A version_synced_dependencies=()
    local in_workspace_dependencies=0 line dependency_name dependency_path manifest_path
    local temp_file
    local dependency_version_pattern='^([[:space:]]*)([[:alnum:]_.-]+)([[:space:]]*=[[:space:]]*\{.*version[[:space:]]*=[[:space:]]*")=[^"]+(".*\}[[:space:]]*)$'

    while IFS= read -r line; do
        if [[ "${line}" == "[workspace.dependencies]" ]]; then
            in_workspace_dependencies=1
            continue
        fi
        if [[ "${line}" == \[* ]]; then
            in_workspace_dependencies=0
        fi
        ((in_workspace_dependencies)) || continue

        if [[ "${line}" =~ ^[[:space:]]*([[:alnum:]_.-]+)[[:space:]]*= ]]; then
            dependency_name="${BASH_REMATCH[1]}"
        else
            continue
        fi
        [[ "${line}" == *"path ="* && "${line}" == *"version ="* ]] || continue
        if [[ "${line}" =~ path[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
            dependency_path="${BASH_REMATCH[1]}"
        else
            continue
        fi

        manifest_path="${WORKSPACE_ROOT}/${dependency_path}/Cargo.toml"
        if [[ -f "${manifest_path}" ]] && grep -Eq '^[[:space:]]*version(\.workspace[[:space:]]*=[[:space:]]*true|[[:space:]]*=[[:space:]]*\{[[:space:]]*workspace[[:space:]]*=[[:space:]]*true[[:space:]]*\})' "${manifest_path}"; then
            version_synced_dependencies["${dependency_name}"]=1
        fi
    done < "${CARGO_TOML}"

    temp_file="$(mktemp)"
    local in_workspace_package=0
    in_workspace_dependencies=0

    while IFS= read -r line; do
        if [[ "${line}" == "[workspace.package]" ]]; then
            in_workspace_package=1
            in_workspace_dependencies=0
        elif [[ "${line}" == "[workspace.dependencies]" ]]; then
            in_workspace_package=0
            in_workspace_dependencies=1
        elif [[ "${line}" == \[* ]]; then
            in_workspace_package=0
            in_workspace_dependencies=0
        fi

        if ((in_workspace_package)) && [[ "${line}" =~ ^[[:space:]]*version[[:space:]]*= ]]; then
            printf 'version = "%s"\n' "${target_version}" >> "${temp_file}"
            continue
        fi

        if ((in_workspace_dependencies)) && [[ "${line}" =~ ${dependency_version_pattern} ]] && [[ -n "${version_synced_dependencies[${BASH_REMATCH[2]}]+x}" ]]; then
            printf '%s%s%s=%s%s\n' \
                "${BASH_REMATCH[1]}" \
                "${BASH_REMATCH[2]}" \
                "${BASH_REMATCH[3]}" \
                "${target_version}" \
                "${BASH_REMATCH[4]}" \
                >> "${temp_file}"
            continue
        fi

        printf '%s\n' "${line}" >> "${temp_file}"
    done < "${CARGO_TOML}"

    mv "${temp_file}" "${CARGO_TOML}"
}

ensure_changelog() {
    local version="$1"
    local base_tag="$2"
    local today version_tag release_link
    local before_links_temp rewritten_temp
    local -a ordered_links=()
    local -A seen_links=()
    local line name url

    if ! grep -Eq "^## \\[${version//./\\.}\\] - " "${CHANGELOG}"; then
        if ! grep -Fxq "## [Unreleased]" "${CHANGELOG}"; then
            echo "CHANGELOG.md is missing the Unreleased section" >&2
            exit 1
        fi

        today="$(date +%F)"
        rewritten_temp="$(mktemp)"
        awk -v version="${version}" -v today="${today}" '
            {
                print
                if (!inserted && $0 == "## [Unreleased]") {
                    print ""
                    print "## [" version "] - " today
                    print ""
                    print "### Changed"
                    print ""
                    print "- TODO: summarize the release changes."
                    print ""
                    inserted = 1
                }
            }
        ' "${CHANGELOG}" > "${rewritten_temp}"
        mv "${rewritten_temp}" "${CHANGELOG}"
    fi

    if ! grep -Fxq "<!-- next-url -->" "${CHANGELOG}"; then
        echo "CHANGELOG.md is missing the next-url marker" >&2
        exit 1
    fi

    version_tag="v${version}"
    if [[ -n "${base_tag}" ]]; then
        release_link="${REPOSITORY_URL}/compare/${base_tag}...${version_tag}"
    else
        release_link="${REPOSITORY_URL}/releases/tag/${version_tag}"
    fi

    ordered_links+=("Unreleased|${REPOSITORY_URL}/compare/${version_tag}...HEAD")
    ordered_links+=("${version}|${release_link}")
    seen_links["Unreleased"]=1
    seen_links["${version}"]=1

    while IFS= read -r line; do
        if [[ "${line}" =~ ^\[([^]]+)\]:[[:space:]]*(.+)$ ]]; then
            name="${BASH_REMATCH[1]}"
            url="${BASH_REMATCH[2]}"
            if [[ -z "${seen_links[${name}]+x}" ]]; then
                ordered_links+=("${name}|${url}")
                seen_links["${name}"]=1
            fi
        fi
    done < <(awk 'found { print } $0 == "<!-- next-url -->" { found = 1 }' "${CHANGELOG}" | tail -n +2)

    before_links_temp="$(mktemp)"
    awk '
        {
            print
            if ($0 == "<!-- next-url -->") {
                exit
            }
        }
    ' "${CHANGELOG}" > "${before_links_temp}"

    rewritten_temp="$(mktemp)"
    {
        cat "${before_links_temp}"
        for line in "${ordered_links[@]}"; do
            IFS='|' read -r name url <<< "${line}"
            printf '[%s]: %s\n' "${name}" "${url}"
        done
    } > "${rewritten_temp}"

    mv "${rewritten_temp}" "${CHANGELOG}"
    rm -f "${before_links_temp}"
}

extract_release_notes() {
    local version="$1"
    local notes

    notes="$(awk -v version="${version}" '
        $0 ~ "^## \\[" version "\\] - " {
            capture = 1
            found = 1
            next
        }
        capture && ($0 ~ "^## \\[" || $0 == "<!-- next-url -->") {
            capture = 0
            exit
        }
        capture {
            print
        }
        END {
            if (!found) {
                exit 2
            }
        }
    ' "${CHANGELOG}")" || {
        echo "CHANGELOG.md is missing the ${version} section" >&2
        exit 1
    }

    if [[ -z "${notes//[$'\t\r\n ']}" ]]; then
        echo "CHANGELOG.md has no notes for ${version}" >&2
        exit 1
    fi

    printf '%s\n' "${notes}" | perl -0pe 's/\A(?:\s*\n)+//; s/(?:\n\s*)+\z/\n/s'
}

compute_next_version() {
    local declared major base_version base_major base_minor base_patch

    declared="$(read_workspace_version)"
    IFS=. read -r major _ _ <<< "${declared}"
    BASE_TAG="$(latest_tag_for_major "${major}")"

    if [[ -z "${BASE_TAG}" ]]; then
        NEXT_VERSION="${declared}"
        BUMP_KIND="initial"
        return
    fi

    if ! git_ref_is_ancestor "${BASE_TAG}" HEAD; then
        echo "${BASE_TAG} is not reachable from HEAD; merge the previous release sync before preparing the next candidate" >&2
        exit 1
    fi

    if [[ -z "$(changed_files "${BASE_TAG}")" ]]; then
        echo "no releaseable changes found since ${BASE_TAG}" >&2
        exit 1
    fi

    base_version="${BASE_TAG#v}"
    IFS=. read -r base_major base_minor base_patch <<< "${base_version}"
    if patch_only_since "${BASE_TAG}"; then
        NEXT_VERSION="${base_major}.${base_minor}.$((base_patch + 1))"
        BUMP_KIND="patch"
    else
        NEXT_VERSION="${base_major}.$((base_minor + 1)).0"
        BUMP_KIND="minor"
    fi
}

command_workspace_metadata() {
    local version
    version="$(read_workspace_version)"
    print_metadata version "${version}" tag_name "v${version}"
}

command_next_version() {
    compute_next_version
    print_metadata \
        version "${NEXT_VERSION}" \
        tag_name "v${NEXT_VERSION}" \
        base_tag "${BASE_TAG}" \
        bump_kind "${BUMP_KIND}"
}

command_prepare_candidate() {
    compute_next_version
    replace_workspace_version "${NEXT_VERSION}"
    ensure_changelog "${NEXT_VERSION}" "${BASE_TAG}"
    print_metadata \
        version "${NEXT_VERSION}" \
        tag_name "v${NEXT_VERSION}" \
        base_tag "${BASE_TAG}" \
        bump_kind "${BUMP_KIND}"
}

command_release_notes() {
    local version="$1"
    shift

    local output=""
    while (($# > 0)); do
        case "$1" in
            --output)
                output="$2"
                shift 2
                ;;
            *)
                echo "unknown argument: $1" >&2
                exit 1
                ;;
        esac
    done

    if [[ -n "${output}" ]]; then
        extract_release_notes "${version}" > "${output}"
    else
        extract_release_notes "${version}"
    fi
}

usage() {
    cat <<'EOF'
Usage:
  packaging/release/release_flow.sh workspace-metadata
  packaging/release/release_flow.sh next-version
  packaging/release/release_flow.sh prepare-candidate
  packaging/release/release_flow.sh release-notes <version> [--output <path>]
EOF
}

if (($# == 0)); then
    usage >&2
    exit 1
fi

command="$1"
shift

case "${command}" in
    workspace-metadata)
        if (($# != 0)); then
            usage >&2
            exit 1
        fi
        command_workspace_metadata
        ;;
    next-version)
        if (($# != 0)); then
            usage >&2
            exit 1
        fi
        command_next_version
        ;;
    prepare-candidate)
        if (($# != 0)); then
            usage >&2
            exit 1
        fi
        command_prepare_candidate
        ;;
    release-notes)
        if (($# < 1)); then
            usage >&2
            exit 1
        fi
        command_release_notes "$@"
        ;;
    *)
        usage >&2
        exit 1
        ;;
esac
