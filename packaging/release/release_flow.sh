#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CARGO_TOML="${WORKSPACE_ROOT}/Cargo.toml"

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

normalize_dependency_manifest() {
    local manifest_path="$1"

    awk '
        function trim(text) {
            sub(/^[[:space:]]+/, "", text)
            sub(/[[:space:]]+$/, "", text)
            return text
        }

        function is_mutable_dependency_key(key) {
            return key == "version" || key == "rev" || key == "tag" || key == "branch" || key == "registry"
        }

        function is_dependency_collection_section(section_name) {
            return section_name ~ /^\[(workspace\.)?(dependencies|dev-dependencies|build-dependencies)\]$/ || section_name ~ /^\[target\..*\.(dependencies|dev-dependencies|build-dependencies)\]$/
        }

        function is_dependency_entry_section(section_name) {
            return section_name ~ /^\[(workspace\.)?(dependencies|dev-dependencies|build-dependencies)\.[^]]+\]$/ || section_name ~ /^\[target\..*\.(dependencies|dev-dependencies|build-dependencies)\.[^]]+\]$/
        }

        function strip_inline_comment(text,    in_double, in_single, character, previous, position) {
            in_double = 0
            in_single = 0
            previous = ""

            for (position = 1; position <= length(text); position++) {
                character = substr(text, position, 1)

                if (character == "\"" && !in_single && previous != "\\") {
                    in_double = !in_double
                } else if (character == "'"'"'" && !in_double) {
                    in_single = !in_single
                } else if (character == "#" && !in_double && !in_single) {
                    return trim(substr(text, 1, position - 1))
                }

                previous = character
            }

            return trim(text)
        }

        function normalize_inline_table_part(part,    separator, key, value) {
            separator = index(part, "=")
            if (!separator) {
                return trim(part)
            }

            key = trim(substr(part, 1, separator - 1))
            value = trim(substr(part, separator + 1))
            if (is_mutable_dependency_key(key)) {
                value = "\"__DEPENDENCY_VALUE__\""
            }
            return key " = " value
        }

        function normalize_inline_table(line,    open_brace, close_brace, prefix, body, in_string, bracket_depth, character, current, count, part, i, j, k, swap) {
            open_brace = index(line, "{")
            close_brace = 0
            for (i = length(line); i > open_brace; i--) {
                if (substr(line, i, 1) == "}") {
                    close_brace = i
                    break
                }
            }

            if (!open_brace || !close_brace) {
                return trim(line)
            }

            prefix = trim(substr(line, 1, open_brace - 1))
            body = substr(line, open_brace + 1, close_brace - open_brace - 1)
            current = ""
            count = 0
            in_string = 0
            bracket_depth = 0

            for (i = 1; i <= length(body); i++) {
                character = substr(body, i, 1)
                if (character == "\"" && substr(body, i - 1, 1) != "\\") {
                    in_string = !in_string
                }

                if (!in_string) {
                    if (character == "[") {
                        bracket_depth++
                    } else if (character == "]") {
                        bracket_depth--
                    } else if (character == "," && bracket_depth == 0) {
                        part = trim(current)
                        if (part != "") {
                            parts[++count] = normalize_inline_table_part(part)
                        }
                        current = ""
                        continue
                    }
                }

                current = current character
            }

            part = trim(current)
            if (part != "") {
                parts[++count] = normalize_inline_table_part(part)
            }

            for (j = 1; j < count; j++) {
                for (k = j + 1; k <= count; k++) {
                    if (parts[j] > parts[k]) {
                        swap = parts[j]
                        parts[j] = parts[k]
                        parts[k] = swap
                    }
                }
            }

            line = prefix " {"
            for (j = 1; j <= count; j++) {
                line = line (j == 1 ? " " : ", ") parts[j]
            }
            line = line " }"

            delete parts
            return line
        }

        function normalize_collection_line(line) {
            line = strip_inline_comment(line)
            if (line ~ /^[A-Za-z0-9_.-]+[[:space:]]*=[[:space:]]*["\047][^"\047]*["\047][[:space:]]*$/) {
                sub(/=[[:space:]]*["\047][^"\047]*["\047]/, "= \"__DEPENDENCY_VALUE__\"", line)
                return line
            }

            if (line ~ /^[A-Za-z0-9_.-]+[[:space:]]*=[[:space:]]*\{.*\}[[:space:]]*$/) {
                return normalize_inline_table(line)
            }

            return line
        }

        function normalize_entry_line(line,    separator, key, value) {
            line = strip_inline_comment(line)
            separator = index(line, "=")
            if (!separator) {
                return line
            }

            key = trim(substr(line, 1, separator - 1))
            value = trim(substr(line, separator + 1))
            if (is_mutable_dependency_key(key)) {
                value = "\"__DEPENDENCY_VALUE__\""
            }
            return key " = " value
        }

        BEGIN {
            section = "__root__"
            section_identity = "__root__"
            section_mode = "other"
        }

        /^[[:space:]]*#/ || /^[[:space:]]*$/ {
            next
        }

        /^[[:space:]]*\[/ {
            section = trim($0)
            if (section ~ /^\[\[/) {
                section_counts[section]++
                section_identity = section "::" section_counts[section]
            } else {
                section_identity = section
            }

            if (is_dependency_collection_section(section)) {
                section_mode = "collection"
            } else if (is_dependency_entry_section(section)) {
                section_mode = "entry"
            } else {
                section_mode = "other"
            }

            print section_identity "\t__section__"
            next
        }

        {
            if (section_mode == "collection") {
                line = normalize_collection_line($0)
            } else if (section_mode == "entry") {
                line = normalize_entry_line($0)
            } else {
                line = strip_inline_comment($0)
            }

            if (line == "") {
                next
            }

            print section_identity "\t" line
        }
    ' "${manifest_path}" | LC_ALL=C sort
}

cargo_manifest_patch_safe() {
    local old_manifest="$1"
    local new_manifest="$2"

    cmp -s <(normalize_dependency_manifest "${old_manifest}") <(normalize_dependency_manifest "${new_manifest}")
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
    local old_manifest new_manifest

    old_manifest="$(mktemp)"
    new_manifest="$(mktemp)"

    if ! git_root show "${base_ref}:${path}" > "${old_manifest}" 2>/dev/null; then
        rm -f "${old_manifest}" "${new_manifest}"
        return 1
    fi

    if [[ ! -f "${WORKSPACE_ROOT}/${path}" ]]; then
        rm -f "${old_manifest}" "${new_manifest}"
        return 1
    fi

    cp "${WORKSPACE_ROOT}/${path}" "${new_manifest}"

    if cargo_manifest_patch_safe "${old_manifest}" "${new_manifest}"; then
        rm -f "${old_manifest}" "${new_manifest}"
        return 0
    fi

    rm -f "${old_manifest}" "${new_manifest}"
    return 1
}

patch_only_since() {
    local base_ref="$1"
    local path

    while IFS= read -r path; do
        [[ -z "${path}" ]] && continue
        case "${path}" in
            Cargo.lock|README.md|deny.toml|renovate.json)
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

    local version_tag="v${version}"
    local prev_tag
    prev_tag=$(git -C "${WORKSPACE_ROOT}" for-each-ref --sort=-creatordate --format='%(refname:strip=2)' refs/tags | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | grep -v "^${version_tag}$" | head -n 1 || true)

    local range
    if [[ -n "${prev_tag}" ]]; then
        range="${prev_tag}..${version_tag}"
    else
        range="${version_tag}"
    fi

    local notes
    notes=$(git -C "${WORKSPACE_ROOT}" log --pretty=format:'- %s (%h)' "${range}" 2>/dev/null || true)

    if [[ -z "${notes//[$'\t\r\n ']}" ]]; then
        notes="- No changes recorded."
    fi

    local rendered
    rendered="$(printf '## %s changes\n\n%s\n' "${version_tag}" "${notes}")"
    if [[ -n "${output}" ]]; then
        printf '%s' "${rendered}" > "${output}"
    else
        printf '%s' "${rendered}"
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

main() {
    local command

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
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
