#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
image=${PANECHORD_LINUX_IMAGE:-panechord-linux}
docker build -f scripts/Dockerfile.linux -t "$image" ..
docker run --rm --init --shm-size=1g --mount type=volume,source=panechord-linux-target,target=/workspace/target "$image"
docker run --rm --init --shm-size=1g --mount type=volume,source=panechord-linux-target,target=/workspace/target "$image" sh panechord/scripts/measure.sh