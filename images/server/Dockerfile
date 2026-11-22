# syntax=docker/dockerfile:1

# The official opkssh, which verifies opkssh certificates. The e2e tests read these lines.
ARG OPKSSH_VERSION=0.20.0
ARG OPKSSH_SHA256_AMD64=1744460367ec1c79f78bde7398737b44556d167957134666107133d6ccadc846
ARG OPKSSH_SHA256_ARM64=554031d65fd10cf4469de00619811c7526a1f1e9d6e1671496ea3d0d8de43e84

FROM scratch AS opkssh-amd64
ARG OPKSSH_VERSION OPKSSH_SHA256_AMD64
ADD --checksum=sha256:${OPKSSH_SHA256_AMD64} \
    https://github.com/openpubkey/opkssh/releases/download/v${OPKSSH_VERSION}/opkssh-linux-amd64 /opkssh

FROM scratch AS opkssh-arm64
ARG OPKSSH_VERSION OPKSSH_SHA256_ARM64
ADD --checksum=sha256:${OPKSSH_SHA256_ARM64} \
    https://github.com/openpubkey/opkssh/releases/download/v${OPKSSH_VERSION}/opkssh-linux-arm64 /opkssh

FROM opkssh-${TARGETARCH} AS opkssh

# The build stage runs on the build host and cross-compiles, so a multi-platform build never
# runs rustc under emulation.
FROM --platform=$BUILDPLATFORM rust:1.100.0-slim-trixie AS build
ARG TARGETARCH
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-gnu > /rust-target; gcc=gcc-x86-64-linux-gnu ;; \
      arm64) echo aarch64-unknown-linux-gnu > /rust-target; gcc=gcc-aarch64-linux-gnu ;; \
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
    --mount=type=cache,target=/src/target,id=corral-server-target,sharing=locked \
    target="$(cat /rust-target)" \
 && cargo build --release --locked --package corral-server --target "$target" \
 && cp "target/$target/release/corral-server" /corral-server
# Corral writes opkssh's server files at startup; opkssh keeps its JWKS cache and log here.
RUN mkdir -p /data /opk/etc /opk/cache \
 && touch /opk/opkssh.log \
 && chmod 0700 /opk/cache \
 && chmod 0660 /opk/opkssh.log

# The web UI is the same files on every platform, so it builds once, natively.
FROM --platform=$BUILDPLATFORM node:22.22.1-trixie-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json web/.npmrc ./
RUN --mount=type=cache,target=/root/.npm,id=corral-npm,sharing=locked npm ci
COPY web ./
RUN npm run build

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /corral-server /usr/local/bin/corral-server
COPY --from=web /web/build /usr/share/corral/web
COPY --from=build --chown=65532:65532 /data /var/lib/corral
COPY --from=opkssh --chmod=0755 /opkssh /usr/local/bin/opkssh
COPY --from=build --chown=65532:65532 /opk/etc /etc/opk
COPY --from=build --chown=65532:65532 /opk/cache /var/cache/opkssh
COPY --from=build --chown=65532:65532 /opk/opkssh.log /var/log/opkssh.log
ENTRYPOINT ["corral-server"]
CMD ["serve"]
