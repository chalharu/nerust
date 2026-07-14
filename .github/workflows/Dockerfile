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

HEALTHCHECK NONE

ARG CARGO_HOME=/usr/local/cargo
ARG RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/usr/local/bin:${PATH}
ENV CARGO_TERM_COLOR=always
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
ENV LIBCUBEB_SYS_USE_PKG_CONFIG=1

RUN <<EOT
  apt-get update -qq
  apt-get install -y --no-install-recommends \
    ca-certificates=20240203 \
    curl=8.5.0-2ubuntu10 \
    gpg=2.4.4-2ubuntu17 \
    build-essential=12.10ubuntu1 \
    cmake=3.28.3-1build7 \
    pkg-config=1.8.1-2build1 \
    clang=1:18.0-59~exp2 \
    libgtk-3-dev=3.24.41-4ubuntu1 \
    libgtk-4-dev=4.14.4+ds-1ubuntu1 \
    libepoxy-dev=1.5.10-1build1 \
    libwebkit2gtk-4.1-dev=2.44.3-0ubuntu0.24.04.1 \
    libxdo-dev=1:3.20211022.1-1.1build2 \
    libasound2-dev=1.2.12-1build1 \
    libcubeb-dev=0.1.0-1build1 \
    libudev-dev=255.4-1build1 \
    openjdk-17-jdk-headless=17.0.12+7-1ubuntu2
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

  # Add Android target
  rustup target add aarch64-linux-android

  # Install commonly used cargo tools (registry still available)
  # renovate: datasource=crate depName=cargo-llvm-cov
  CARGO_LLVM_COV_VERSION="0.8.7"
  cargo install cargo-llvm-cov --locked --version "=${CARGO_LLVM_COV_VERSION}"

  # renovate: datasource=crate depName=cargo-ndk
  CARGO_NDK_VERSION="4.1.2"
  cargo install cargo-ndk --locked --version "=${CARGO_NDK_VERSION}"

  # Clean up cargo registry to reduce image size
  rm -rf /usr/local/cargo/registry
EOT

RUN groupadd -g 1001 runner \
    && useradd -m -u 1001 runner -g 1001 \
    && chown -R runner:runner /usr/local/cargo /usr/local/rustup
USER runner
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/usr/local/bin:${PATH}

LABEL org.opencontainers.image.source="https://github.com/chalharu/nerust"
LABEL org.opencontainers.image.description="nerust CI image"
