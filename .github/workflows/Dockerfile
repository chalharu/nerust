# syntax=docker/dockerfile:1
FROM ubuntu:24.04

SHELL ["/bin/bash", "-euxo", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# renovate: datasource=github-releases depName=rui314/mold
ARG MOLD_VERSION=2.36.0
# renovate: datasource=github-releases depName=rust-lang/rust
ARG RUST_TOOLCHAIN=1.97.0
# renovate: datasource=github-releases depName=rust-lang/rustup
ARG RUSTUP_VERSION=1.29.0

# Sets /tmp if GITHUB_WORKSPACE is not set.
ARG GITHUB_WORKSPACE=/tmp

USER 1000

HEALTHCHECK NONE

ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/usr/local/bin:${PATH}
ENV CARGO_TERM_COLOR=always
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
ENV LIBCUBEB_SYS_USE_PKG_CONFIG=1

RUN <<EOT
  apt-get update -qq
  apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    build-essential \
    pkg-config \
    clang \
    libgtk-3-dev \
    libgtk-4-dev \
    libepoxy-dev \
    libwebkit2gtk-4.1-dev \
    libxdo-dev \
    libasound2-dev \
    libcubeb-dev \
    libudev-dev
  apt-get clean
  rm -rf /var/lib/apt/lists/*
EOT

WORKDIR /tmp

RUN <<EOT
  # Install mold
  curl -fsSL "https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-$(uname -m)-linux.tar.gz" \
    | tar -C /usr/local --strip-components=1 -xzf -

  RUST_TARGET_TRIPLE=$(uname -m)-unknown-linux-gnu

  # Install Rust toolchain via rustup
  curl -fsSL "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${RUST_TARGET_TRIPLE}/rustup-init" \
    -o /tmp/rustup-init

  curl -fsSL "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${RUST_TARGET_TRIPLE}/rustup-init.sha256" \
    | sha256sum --check
  chmod +x /tmp/rustup-init
  /tmp/rustup-init -y --no-modify-path --default-toolchain "${RUST_TOOLCHAIN}" \
    --profile minimal \
    --component rustfmt,clippy,llvm-tools-preview
  rm -f /tmp/rustup-init

  rm -rf /root/.cargo/registry
EOT

WORKDIR ${GITHUB_WORKSPACE}

LABEL org.opencontainers.image.source="https://github.com/chalharu/nerust"
LABEL org.opencontainers.image.description="nerust CI image"
