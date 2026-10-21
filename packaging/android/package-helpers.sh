#!/usr/bin/env bash
# Pure packaging operations, sourceable without invoking Cargo, Gradle or an SDK.

verify_native_library() {
    local reader="$1" library="$2" budget="$3" sections size
    # A failed reader is not evidence of an absence of DWARF. Do not put it in
    # an `if reader | grep` pipeline: that hides failure (and SIGPIPE) statuses.
    if ! sections="$("${reader}" -S "${library}")"; then
        echo "cannot inspect native sections: ${library}" >&2
        return 1
    fi
    if grep -Eq '\.debug_|\.zdebug_' <<< "${sections}"; then
        echo "${library} contains DWARF sections; release strip is not effective" >&2
        return 1
    fi
    if [[ ! "${budget}" =~ ^[1-9][0-9]*$ ]]; then
        echo "invalid native byte budget: ${budget}" >&2
        return 1
    fi
    size="$(wc -c < "${library}")" || return 1
    if (( size > budget )); then
        echo "${library} is ${size} bytes, over the ${budget} budget" >&2
        return 1
    fi
    echo "${library}: ${size} bytes, no DWARF"
}

publish_apk() (
    local source="$1" output="$2" name="$3"
    mkdir -p "${output}" || return 1
    cp "${source}" "${output}/${name}" || return 1
    # Both generation and verification use a basename relative to OUT_DIR.
    # This works for relative paths and keeps checksums usable after download.
    cd "${output}" || return 1
    sha256sum "${name}" > "${name}.sha256" || return 1
    sha256sum -c "${name}.sha256"
)