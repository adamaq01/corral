# syntax=docker/dockerfile:1
# Built from the repository root: docker build --file images/overlay/Dockerfile .
#
# Corral's pieces for any base image. Each engine mounts this image at /opt/corral, so its root
# is that directory, and every binary in it is static: it runs whatever libc the base has, or none.

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

# The C programs, built statically against musl for the target platform.
FROM alpine:3.24 AS build
ARG TARGETARCH
ARG SUDO_VERSION=1.9.17p2
ARG SUDO_SHA256=4a38a1ab3adb1199257edc2a7c4a2bd714665eb605b04368843b06dada2cfcfb
ARG OPENSSH_VERSION=10.6p2
ARG OPENSSH_SHA256=9ae44872090cf2f06b3cf9ff147e7f16f309671340763ed2f99248398ae29d5b
ARG BASH_VERSION=5.3
ARG BASH_SHA256=0d5cd86965f869a26cf64f4b71be7b96f90a3ba8b3d74e27e8e9d9d5550f31ba
ARG HERDR_CORE_VERSION=0.1.21
ARG HERDR_CORE_SHA256_AMD64=78424755c10a40dde38a0bdb55c20ac19706e9ae05ed68fc2acf1fcff0d941c1
ARG HERDR_CORE_SHA256_ARM64=e844a2bfab48a226097679d076bc83042fe57fccef5387ef802e5b24e78b529a
ARG P=/opt/corral
RUN apk add --no-cache bash build-base linux-headers curl perl busybox-static tini-static \
      openssl-dev openssl-libs-static ncurses-dev ncurses-static
# sudo's libtool needs a POSIX shell with `+=`, which busybox's lacks.
ENV CONFIG_SHELL=/bin/bash
WORKDIR /src

RUN curl -fsSL -o sudo.tar.gz "https://www.sudo.ws/dist/sudo-$SUDO_VERSION.tar.gz" \
 && echo "$SUDO_SHA256  sudo.tar.gz" | sha256sum -c - \
 && tar -xzf sudo.tar.gz && cd "sudo-$SUDO_VERSION" \
 && ./configure --prefix=$P --sysconfdir=$P/etc --disable-shared --enable-static-sudoers \
      --without-pam --disable-nls --without-sendmail --with-rundir=/run/corral-sudo \
      --with-vardir=/var/lib/corral-sudo \
      --with-secure-path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$P/bin:$P/fallback" \
      LDFLAGS=-static >/dev/null \
 && make -j"$(nproc)" LDFLAGS=-all-static >/dev/null \
 && install -D -m 4755 src/sudo /out$P/bin/sudo

RUN curl -fsSL -o openssh.tar.gz \
      "https://cdn.openbsd.org/pub/OpenBSD/OpenSSH/portable/openssh-$OPENSSH_VERSION.tar.gz" \
 && echo "$OPENSSH_SHA256  openssh.tar.gz" | sha256sum -c - \
 && tar -xzf openssh.tar.gz && cd "openssh-$OPENSSH_VERSION" \
 && ./configure --prefix=$P --sysconfdir=$P/etc --with-privsep-path=/var/empty \
      --with-privsep-user=nobody --without-pam --without-zlib --with-ldflags=-static \
      --disable-lastlog --disable-utmp --disable-wtmp --disable-utmpx --disable-wtmpx \
      --with-default-path=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$P/bin:$P/fallback \
      --with-pid-dir=/run >/dev/null \
 && make -j"$(nproc)" sshd sshd-session sshd-auth sftp-server ssh-keygen >/dev/null \
 && install -D -m 0755 sshd /out$P/sbin/sshd \
 && install -D -m 0755 -t /out$P/libexec sshd-session sshd-auth sftp-server \
 && mkdir -p /out$P/etc \
 && ./ssh-keygen -q -t ed25519 -N '' -C corral-sandbox -f /out$P/etc/ssh_host_ed25519_key \
 && rm /out$P/etc/ssh_host_ed25519_key.pub

# GNU's own servers often time out from CI runners; the checksum makes any mirror safe.
RUN for url in "https://mirrors.kernel.org/gnu/bash/bash-$BASH_VERSION.tar.gz" \
      "https://ftpmirror.gnu.org/bash/bash-$BASH_VERSION.tar.gz" \
      "https://ftp.gnu.org/gnu/bash/bash-$BASH_VERSION.tar.gz"; do \
      curl -fsSL --retry 3 --connect-timeout 30 -o bash.tar.gz "$url" && break; \
    done \
 && echo "$BASH_SHA256  bash.tar.gz" | sha256sum -c - \
 && tar -xzf bash.tar.gz && cd "bash-$BASH_VERSION" \
 && ./configure --enable-static-link --without-bash-malloc --disable-nls >/dev/null \
 && make -j"$(nproc)" >/dev/null \
 && install -D -m 0755 bash /out$P/bin/bash

RUN case "$TARGETARCH" in \
      amd64) arch=x86_64; sha=$HERDR_CORE_SHA256_AMD64 ;; \
      arm64) arch=aarch64; sha=$HERDR_CORE_SHA256_ARM64 ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o herdr-core.tar.gz \
      "https://releases.herdr.dev/core/$HERDR_CORE_VERSION/herdr-core-linux-$arch.tar.gz" \
 && echo "$sha  herdr-core.tar.gz" | sha256sum -c - \
 && mkdir -p /out$P/share/doc/herdr-core \
 && tar -xzf herdr-core.tar.gz -C /out$P/share/doc/herdr-core --no-same-owner \
      LICENSE THIRD-PARTY-NOTICES \
 && tar -xzf herdr-core.tar.gz -C /out$P/bin --no-same-owner herdr-core \
 && ln -s herdr-core /out$P/bin/hc

# busybox is the overlay's own shell and the fallback for every command the base lacks, at the
# end of PATH.
RUN install -m 0755 /bin/busybox.static /out$P/bin/busybox \
 && install -m 0755 /sbin/tini-static /out$P/bin/tini \
 && ln -s busybox /out$P/bin/sh \
 && mkdir -p /out$P/fallback \
 && for applet in $(/bin/busybox.static --list); do \
      ln -s ../bin/busybox "/out$P/fallback/$applet"; \
    done \
 && strip /out$P/bin/sudo /out$P/bin/bash /out$P/sbin/sshd /out$P/libexec/*

COPY --from=corral-command /corral /out/opt/corral/bin/corral
COPY --chmod=0755 images/overlay/corral-init images/overlay/corral-shell /out/opt/corral/bin/
COPY --chmod=0755 images/overlay/corral-sshd /out/opt/corral/libexec/
COPY images/overlay/motd /out/opt/corral/etc/
# Every sandbox's sshd has this host key, readable by the sandbox user it runs as: the gate opens
# the stream it speaks over through the engine, so it checks no host key, and the key is no
# secret. sshd refuses a key that others can read only when the key is its own user's.
RUN chmod 0644 /out/opt/corral/etc/ssh_host_ed25519_key
COPY --chmod=0440 images/overlay/sudoers /out/opt/corral/etc/sudoers
COPY images/overlay/skel/ /out/opt/corral/etc/skel/

FROM scratch
COPY --from=build /out/opt/corral/ /
