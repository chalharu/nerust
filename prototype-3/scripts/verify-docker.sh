#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
image=${WAKUI_LINUX_IMAGE:-wakui3-linux}
docker build -f scripts/Dockerfile.linux -t "$image" .
docker run --rm --init --shm-size=1g --mount type=volume,source=wakui3-linux-target,target=/workspace/target "$image"
docker run --rm --init --shm-size=1g --mount type=volume,source=wakui3-linux-target,target=/workspace/target "$image" sh scripts/measure.sh