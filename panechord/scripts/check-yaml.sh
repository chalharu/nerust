#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$#" -eq 0 ]; then set -- check --workspace; fi
export RUSTC_WRAPPER="$PWD/scripts/yaml-rustc.py"
exec cargo "$@"