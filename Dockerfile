# syntax=docker/dockerfile:1

# Keep builder/runtime on the same Debian release for glibc compatibility.
# Digests were verified against the official Docker registry on 2026-10-24.
FROM rust:1.99.1-trixie@sha256:931c2c0531a8b143e044f0f3914dfe802f15bb3d1955f0b657e9359327ac2ffc AS build
WORKDIR /build
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ ./src/
COPY migrations/ ./migrations/
COPY docs/openapi.json docs/catalog.seed.json ./docs/

# Copy binaries out of the cache mount so later stages receive real artifacts.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release --bin careerbridge-be --bin migrate \
    && install -D -m 0555 target/release/careerbridge-be /out/careerbridge-be \
    && install -D -m 0555 target/release/migrate /out/migrate

# Credential-free tests. Database/provider tests remain separate explicit checks.
FROM build AS test
COPY tests/ ./tests/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo test --locked

FROM debian:trixie-slim@sha256:62bd423aeee912639ca8ee681c2b762cb3e25137e0a92b20be455898a9f034ad AS runtime-base
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl libstdc++6 libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 careerbridge \
    && useradd --uid 10001 --gid 10001 --no-create-home \
        --home-dir /app --shell /usr/sbin/nologin careerbridge \
    && mkdir -p /app/model-cache \
    && chown 10001:10001 /app/model-cache
WORKDIR /app

# Run separately with only MIGRATION_DATABASE_URL and schema-owner credentials.
FROM runtime-base AS migrate
COPY --from=build --chmod=0555 /out/migrate /usr/local/bin/migrate
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/migrate"]

# Default image: one process/model shared by the two Actix HTTP workers.
FROM runtime-base AS runtime
COPY --from=build --chmod=0555 /out/careerbridge-be /usr/local/bin/careerbridge-be
ENV HOST=0.0.0.0 \
    PORT=8080 \
    MODEL_CACHE_DIR=/app/model-cache \
    RAYON_NUM_THREADS=2 \
    OMP_NUM_THREADS=2 \
    RUST_LOG=info
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=180s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 4 "http://127.0.0.1:${PORT}/ready" || exit 1
ENTRYPOINT ["/usr/local/bin/careerbridge-be"]
