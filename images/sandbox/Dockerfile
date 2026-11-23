# syntax=docker/dockerfile:1
# Built from the repository root: docker build --file images/sandbox/Dockerfile .

# The corral command, a static musl binary. The stage runs on the build host and cross-compiles,
# so a multi-platform build never runs rustc under emulation.
FROM --platform=$BUILDPLATFORM rust:1.100.0-slim-trixie AS corral-command
ARG TARGETARCH
# The glibc cross compilers build ring's C code and link for musl, with the musl libc and start
# files rustc brings.
ENV CC_x86_64_unknown_linux_musl=x86_64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_musl=aarch64-linux-gnu-gcc \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gnu-gcc
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl > /rust-target; gcc=gcc-x86-64-linux-gnu ;; \
      arm64) echo aarch64-unknown-linux-musl > /rust-target; gcc=gcc-aarch64-linux-gnu ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && if [ "$(dpkg --print-architecture)" != "$TARGETARCH" ]; then \
      apt-get update \
      && apt-get install -y --no-install-recommends "$gcc" "libc6-dev-$TARGETARCH-cross" \
      && rm -rf /var/lib/apt/lists/*; \
    fi
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
RUN rustup target add "$(cat /rust-target)"
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=corral-cargo-registry,sharing=locked \
    --mount=type=cache,target=/src/target,id=corral-command-target,sharing=locked \
    target="$(cat /rust-target)" \
 && cargo build --release --locked --package corral-cli --target "$target" \
      --config profile.release.strip=true \
 && cp "target/$target/release/corral" /corral

FROM ubuntu:24.04

ARG TARGETARCH
ARG DEBIAN_FRONTEND=noninteractive

ARG NODE_VERSION=26.13.1
ARG NODE_SHA256_AMD64=518674de2b621728a5e4486f5fb3b7d2900bb3cc4aa97969edaa463db99f9778
ARG NODE_SHA256_ARM64=80a330b35415382878df18d41afbd106ea89bfb3be0087d6608e289a14c5cc7d
ARG HERDR_CORE_VERSION=0.1.21
ARG HERDR_CORE_SHA256_AMD64=78424755c10a40dde38a0bdb55c20ac19706e9ae05ed68fc2acf1fcff0d941c1
ARG HERDR_CORE_SHA256_ARM64=e844a2bfab48a226097679d076bc83042fe57fccef5387ef802e5b24e78b529a
ARG CLAUDE_CODE_VERSION=2.1.330
ARG CODEX_VERSION=0.171.0

ENV LANG=C.UTF-8
# The user's login shell, which sshd sets but a Kubernetes exec does not.
ENV SHELL=/bin/bash

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git sudo tini build-essential procps openssh-client openssh-server python3 \
 && rm -rf /var/lib/apt/lists/* /etc/ssh/ssh_host_*

RUN userdel --remove ubuntu \
 && groupadd --gid 1000 sandbox \
 && useradd --uid 1000 --gid 1000 --shell /bin/bash --home-dir /home/sandbox --no-create-home sandbox \
 && install -d -o 1000 -g 1000 -m 0750 /home/sandbox \
 && mkdir -p /etc/corral \
 && cp -R /etc/skel /etc/corral/skel \
 && rm -rf /etc/update-motd.d /etc/legal \
 && sed -i '/pam_motd/d' /etc/pam.d/sshd

RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) node_arch=x64; node_sha=$NODE_SHA256_AMD64; herdr_arch=x86_64; herdr_sha=$HERDR_CORE_SHA256_AMD64 ;; \
      arm64) node_arch=arm64; node_sha=$NODE_SHA256_ARM64; herdr_arch=aarch64; herdr_sha=$HERDR_CORE_SHA256_ARM64 ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/node.tar.gz "https://nodejs.org/dist/v$NODE_VERSION/node-v$NODE_VERSION-linux-$node_arch.tar.gz"; \
    echo "$node_sha  /tmp/node.tar.gz" | sha256sum -c -; \
    tar -xzf /tmp/node.tar.gz -C /usr/local --strip-components=1 --no-same-owner; \
    curl -fsSL -o /tmp/herdr-core.tar.gz "https://releases.herdr.dev/core/$HERDR_CORE_VERSION/herdr-core-linux-$herdr_arch.tar.gz"; \
    echo "$herdr_sha  /tmp/herdr-core.tar.gz" | sha256sum -c -; \
    mkdir -p /usr/local/share/doc/herdr-core; \
    tar -xzf /tmp/herdr-core.tar.gz -C /usr/local/share/doc/herdr-core --no-same-owner LICENSE THIRD-PARTY-NOTICES; \
    tar -xzf /tmp/herdr-core.tar.gz -C /usr/local/bin --no-same-owner herdr-core; \
    ln -s herdr-core /usr/local/bin/hc; \
    npm install --global --allow-scripts=@anthropic-ai/claude-code \
      "@anthropic-ai/claude-code@$CLAUDE_CODE_VERSION" "@openai/codex@$CODEX_VERSION"; \
    npm cache clean --force; \
    rm -rf /tmp/* /root/.npm

COPY images/sandbox/motd /etc/motd
COPY images/sandbox/corral-motd.sh /etc/profile.d/
COPY images/sandbox/sshd_config /etc/ssh/sshd_config
COPY --chmod=0440 images/sandbox/sudoers /etc/sudoers.d/sandbox
COPY --chmod=0755 images/sandbox/corral-home-init images/sandbox/corral-entrypoint /usr/local/bin/
COPY --from=corral-command /corral /usr/local/bin/corral

USER 1000
WORKDIR /home/sandbox
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/corral-entrypoint"]
