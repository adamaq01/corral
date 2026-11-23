# syntax=docker/dockerfile:1
# Built from the repository root: docker build --file images/sandbox/Dockerfile .
#
# Ubuntu with the tools of the default catalog entry. Corral's own pieces, Herdr included, come
# from the overlay every sandbox mounts, so this image has none of them.
FROM ubuntu:24.04

ARG TARGETARCH
ARG DEBIAN_FRONTEND=noninteractive

ARG NODE_VERSION=26.13.1
ARG NODE_SHA256_AMD64=518674de2b621728a5e4486f5fb3b7d2900bb3cc4aa97969edaa463db99f9778
ARG NODE_SHA256_ARM64=80a330b35415382878df18d41afbd106ea89bfb3be0087d6608e289a14c5cc7d
ARG CLAUDE_CODE_VERSION=2.1.330
ARG CODEX_VERSION=0.171.0

ENV LANG=C.UTF-8

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential procps openssh-client python3 \
 && rm -rf /var/lib/apt/lists/*

RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) node_arch=x64; node_sha=$NODE_SHA256_AMD64 ;; \
      arm64) node_arch=arm64; node_sha=$NODE_SHA256_ARM64 ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/node.tar.gz "https://nodejs.org/dist/v$NODE_VERSION/node-v$NODE_VERSION-linux-$node_arch.tar.gz"; \
    echo "$node_sha  /tmp/node.tar.gz" | sha256sum -c -; \
    tar -xzf /tmp/node.tar.gz -C /usr/local --strip-components=1 --no-same-owner; \
    npm install --global --allow-scripts=@anthropic-ai/claude-code \
      "@anthropic-ai/claude-code@$CLAUDE_CODE_VERSION" "@openai/codex@$CODEX_VERSION"; \
    npm cache clean --force; \
    rm -rf /tmp/* /root/.npm
