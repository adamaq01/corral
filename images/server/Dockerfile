# syntax=docker/dockerfile:1

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
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=corral-cargo-registry \
    --mount=type=cache,target=/src/target,id=corral-server-target \
    target="$(cat /rust-target)" \
 && cargo build --release --locked --package corral-server --target "$target" \
 && cp "target/$target/release/corral-server" /corral-server
RUN mkdir /data

# The web UI is the same files on every platform, so it builds once, natively.
FROM --platform=$BUILDPLATFORM node:22.22.1-trixie-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json web/.npmrc ./
RUN --mount=type=cache,target=/root/.npm,id=corral-npm npm ci
COPY web ./
RUN npm run build

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /corral-server /usr/local/bin/corral-server
COPY --from=web /web/build /usr/share/corral/web
COPY --from=build --chown=65532:65532 /data /var/lib/corral
ENTRYPOINT ["corral-server"]
CMD ["serve"]
