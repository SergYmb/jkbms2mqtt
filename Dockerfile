# ─── Stage 1: builder (pinned to BUILDPLATFORM, cross-compiles to TARGET) ────
FROM --platform=$BUILDPLATFORM ghcr.io/rust-cross/cargo-zigbuild:latest AS builder
WORKDIR /src

# Docker buildx injects these automatically per platform.
ARG TARGETARCH
ARG TARGETVARIANT

# Resolve $TARGETARCH/$TARGETVARIANT → Rust triple, install the target.
# The triple is written to /rust-target so the build step below can read it
# back without re-running the case block.
RUN set -eu; \
    case "${TARGETARCH}/${TARGETVARIANT:-}" in \
      amd64/*)   TRIPLE=x86_64-unknown-linux-musl ;; \
      arm64/*)   TRIPLE=aarch64-unknown-linux-musl ;; \
      arm/v7)    TRIPLE=armv7-unknown-linux-musleabihf ;; \
      arm/v6)    TRIPLE=arm-unknown-linux-musleabihf ;; \
      *) echo "unsupported TARGETARCH/TARGETVARIANT: ${TARGETARCH}/${TARGETVARIANT:-}"; exit 1 ;; \
    esac; \
    echo "$TRIPLE" > /rust-target; \
    rustup target add "$TRIPLE"

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# cargo-registry: crate source cache, shared across archs.
# cargo-target:   per-arch (${TARGETARCH}${TARGETVARIANT}) to avoid contention
#                 between concurrent multi-arch buildx jobs.
RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-target-${TARGETARCH}${TARGETVARIANT},target=/src/target,sharing=locked \
    set -eu; \
    TRIPLE=$(cat /rust-target); \
    cargo zigbuild --release --target "$TRIPLE"; \
    cp "target/${TRIPLE}/release/jkbms2mqtt" /jkbms2mqtt

# ─── Stage 2: runtime (runs on TARGETPLATFORM) ───────────────────────────────
FROM alpine:3
RUN adduser -D -u 1000 jkbms2mqtt
COPY --from=builder /jkbms2mqtt /usr/local/bin/jkbms2mqtt
USER jkbms2mqtt
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s \
    CMD ["/usr/local/bin/jkbms2mqtt", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/jkbms2mqtt", "--healthcheck-server"]
