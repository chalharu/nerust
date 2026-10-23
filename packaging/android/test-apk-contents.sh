#!/usr/bin/env bash
# Probes for verify_apk_contents (NER-314). Manual run:
#   bash packaging/android/test-apk-contents.sh
# Wired into CI: check-contract.sh "shell probes" phase + the lint job's
# explicit shell-probes step in .github/workflows/ci.yml.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=package-helpers.sh
source "${SCRIPT_DIR}/package-helpers.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
PASS=0
# The release APK must carry the Rust cdylib (crate lib name "main",
# loaded via System.loadLibrary("main")); the gate below fails the
# build instead of shipping an APK without it (NER-314).
EXPECTED_LIB="libmain.so"
pass() {
    PASS=$((PASS + 1))
    echo "ok: $1"
}
fail() {
    echo "FAIL: $1" >&2
    exit 1
}

# Valid APK carrying the expected library passes.
python3 - "${TMP}/good.apk" "${EXPECTED_LIB}" <<'EOF'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as apk:
    apk.writestr("lib/arm64-v8a/" + sys.argv[2], b"\x7fELF fake")
    apk.writestr("classes.dex", b"dex")
EOF
verify_apk_contents "${TMP}/good.apk" "${EXPECTED_LIB}" >/dev/null \
    || fail "valid APK with the native lib was rejected"
pass "valid APK with the native lib passes"

# Valid zip without the library fails: wrong artifact, not a transfer error.
python3 - "${TMP}/nolib.apk" <<'EOF'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as apk:
    apk.writestr("classes.dex", b"dex")
EOF
if verify_apk_contents "${TMP}/nolib.apk" "${EXPECTED_LIB}" >/dev/null 2>&1; then
    fail "APK without the native lib passed"
fi
pass "APK without the native lib fails"

# Corrupt bytes fail instead of passing the checksum-shaped gate.
printf 'not a zip at all' >"${TMP}/corrupt.apk"
if verify_apk_contents "${TMP}/corrupt.apk" "${EXPECTED_LIB}" >/dev/null 2>&1; then
    fail "corrupt APK passed"
fi
pass "corrupt APK fails"

# Missing or empty APK fails.
if verify_apk_contents "${TMP}/absent.apk" "${EXPECTED_LIB}" >/dev/null 2>&1; then
    fail "missing APK passed"
fi
: >"${TMP}/empty.apk"
if verify_apk_contents "${TMP}/empty.apk" "${EXPECTED_LIB}" >/dev/null 2>&1; then
    fail "empty APK passed"
fi
pass "missing and empty APKs fail"

echo "apk-contents probes: OK (${PASS} scenarios)"
