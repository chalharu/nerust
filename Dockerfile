# syntax=docker/dockerfile:1
FROM ubuntu:24.04

SHELL ["/bin/bash", "-euxo", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# renovate: datasource=github-releases depName=rui314/mold
ARG MOLD_VERSION=2.36.0
# renovate: datasource=github-releases depName=rust-lang/rust
ARG RUST_TOOLCHAIN=1.97.0

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

  # Install mold
  curl -fsSL "https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-$(uname -m)-linux.tar.gz" \
    | tar -C /usr/local --strip-components=1 -xzf -

  # Install Rust toolchain via rustup
  # renovate: datasource=github-releases depName=rust-lang/rustup
  RUSTUP_VERSION=1.28.1
  curl -fsSL "https://github.com/rust-lang/rustup/releases/download/${RUSTUP_VERSION}/rustup-init-$(uname -m)-unknown-linux-gnu" \
    -o /tmp/rustup-init
  echo "$(curl -fsSL "https://github.com/rust-lang/rustup/releases/download/${RUSTUP_VERSION}/rustup-init-$(uname -m)-unknown-linux-gnu.sha256") /tmp/rustup-init" \
    | sha256sum --check
  chmod +x /tmp/rustup-init
  /tmp/rustup-init -y --no-modify-path --default-toolchain "${RUST_TOOLCHAIN}" \
    --profile minimal \
    --component rustfmt,clippy,llvm-tools-preview
  rm -f /tmp/rustup-init

  apt-get clean
  rm -rf /var/lib/apt/lists/*
  rm -rf /root/.cargo/registry
EOT

ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/usr/local/bin:${PATH}
ENV CARGO_TERM_COLOR=always
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=clang
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
ENV LIBCUBEB_SYS_USE_PKG_CONFIG=1

RUN rustc --version && cargo --version && mold --version

LABEL org.opencontainers.image.source="https://github.com/chalharu/nerust"
LABEL org.opencontainers.image.description="nerust CI image"
