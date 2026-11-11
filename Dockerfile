# syntax=docker/dockerfile:1

# Keep builder/runtime on the same Debian release for glibc compatibility.
# Digests were verified against the official Docker registry on 2026-10-24.
FROM rust:1.99.1-trixie@sha256:931c2c0531a8b143e044f0f3914dfe802f15bb3d1955f0b657e9359327ac2ffc AS build
WORKDIR /build
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates cmake pkg-config curl jq \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ ./src/
COPY migrations/ ./migrations/
COPY docs/catalog.seed.json ./docs/
COPY docs/swagger-ui/ ./docs/swagger-ui/

# Copy binaries out of the cache mount so later stages receive real artifacts.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release --bin careerbridge-be --bin migrate --bin verify-model --bin job-import --bin skillhub-import --bin diagnose-cv \
    && install -D -m 0555 target/release/careerbridge-be /out/careerbridge-be \
    && install -D -m 0555 target/release/migrate /out/migrate \
    && install -D -m 0555 target/release/verify-model /out/verify-model \
    && install -D -m 0555 target/release/job-import /out/job-import \
    && install -D -m 0555 target/release/skillhub-import /out/skillhub-import \
    && install -D -m 0555 target/release/diagnose-cv /out/diagnose-cv

# Fetch immutable, hash-verified model assets without application credentials.
FROM build AS model-assets
COPY docker/model-artifacts.json /build/docker/model-artifacts.json
RUN <<'SH'
set -eu
manifest=/build/docker/model-artifacts.json
model=$(jq -er '.model' "$manifest")
revision=$(jq -er '.revision' "$manifest")
test "$model" = 'intfloat/multilingual-e5-small'
test "$revision" = '614241f622f53c4eeff9890bdc4f31cfecc418b3'
cache=/out/model-cache/models--intfloat--multilingual-e5-small
snapshot="$cache/snapshots/$revision"
jq -er '.files[] | [.sha256, (.bytes|tostring), .file] | @tsv' "$manifest" > /tmp/model-files
while read -r digest size file; do
    case "$file" in
        config.json|onnx/model.onnx|special_tokens_map.json|tokenizer.json|tokenizer_config.json) ;;
        *) echo 'Unexpected model artifact path' >&2; exit 1 ;;
    esac
    target="$snapshot/$file"
    mkdir -p "$(dirname "$target")"
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --fail --silent --show-error --location --retry 5 \
        --connect-timeout 20 --max-time 600 \
        "https://huggingface.co/$model/resolve/$revision/$file" --output "$target"
    test "$(wc -c < "$target")" -eq "$size"
    printf '%s  %s\n' "$digest" "$target" | sha256sum --check --strict -
done < /tmp/model-files
mkdir -p "$cache/refs"
printf '%s' "$revision" > "$cache/refs/main"
SH

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

# Run separately with only SUPABASE_DB_URL and schema-owner credentials.
FROM runtime-base AS migrate
COPY --from=build --chmod=0555 /out/migrate /usr/local/bin/migrate
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/migrate"]

# Default image: one process/model shared by the two Actix HTTP workers.
FROM runtime-base AS runtime
COPY --from=build --chmod=0555 /out/careerbridge-be /usr/local/bin/careerbridge-be
COPY --from=build --chmod=0555 /out/verify-model /usr/local/bin/verify-model
COPY --from=build --chmod=0555 /out/job-import /usr/local/bin/job-import
COPY --from=build --chmod=0555 /out/skillhub-import /usr/local/bin/skillhub-import
COPY --from=build --chmod=0555 /out/diagnose-cv /usr/local/bin/diagnose-cv
COPY --from=model-assets --chown=10001:10001 /out/model-cache/ /app/model-cache/
ENV EMBEDDING_MODEL_REVISION=614241f622f53c4eeff9890bdc4f31cfecc418b3 \
    HOST=0.0.0.0 \
    PORT=8080 \
    MODEL_CACHE_DIR=/app/model-cache \
    RAYON_NUM_THREADS=2 \
    OMP_NUM_THREADS=2 \
    RUST_LOG=info
USER 10001:10001
RUN --network=none /usr/local/bin/verify-model
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=180s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 4 "http://127.0.0.1:${PORT}/ready" || exit 1
ENTRYPOINT ["/usr/local/bin/careerbridge-be"]
