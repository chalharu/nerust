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

# Real artifact check, not a transfer checksum: the APK must be a valid
# zip containing the expected native library for at least one ABI.
# A corrupt download and a wrong-artifact publish both fail here.
verify_apk_contents() {
    local apk="$1" expected="$2"
    [[ -s "${apk}" ]] || {
        echo "missing or empty APK: ${apk}" >&2
        return 1
    }
    python3 - "${apk}" "${expected}" <<'EOF' || return 1
import sys
import zipfile
apk, expected = sys.argv[1], sys.argv[2]
try:
    names = zipfile.ZipFile(apk).namelist()
except zipfile.BadZipFile as error:
    print(f"APK is not a valid zip: {apk}: {error}", file=sys.stderr)
    raise SystemExit(1)
hits = [name for name in names if name.endswith("/" + expected)]
if not hits:
    print(f"APK has no {expected} under lib/<abi>/", file=sys.stderr)
    raise SystemExit(1)
print(f"APK contents OK: {expected} in {sorted({name.split('/')[1] for name in hits})}")
EOF
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