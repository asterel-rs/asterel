# syntax=docker/dockerfile:1

# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.95.0-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

ENV PROTOC=/usr/bin/protoc
RUN protoc --version

# 1. Copy manifests to cache dependencies
COPY Cargo.toml Cargo.lock ./
# Create dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN --mount=type=cache,target=/app/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git/db,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release --locked
RUN rm -rf src

# 2. Copy source code
COPY . .
# Touch main.rs to force rebuild
RUN touch src/main.rs
RUN --mount=type=cache,target=/app/target,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git/db,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release --locked && \
    install -m 0755 target/release/asterel /usr/local/bin/asterel && \
    strip /usr/local/bin/asterel

# ── Stage 2: Permissions & Config Prep ───────────────────────
FROM busybox:latest AS permissions
# Create directory structure (simplified workspace path)
RUN mkdir -p /asterel-data/.asterel /asterel-data/workspace

# Create minimal container config.
# NOTE: The image binds inside the container so Docker port publishing can work,
# but examples should publish the host port on loopback or place a trusted reverse
# proxy in front. Provider configuration must be done via environment variables
# at runtime.
RUN cat > /asterel-data/.asterel/config.toml << 'EOF'
workspace_dir = "/asterel-data/workspace"
config_path = "/asterel-data/.asterel/config.toml"
api_key = ""
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4-20250514"
default_temperature = 0.7

[gateway]
port = 3000
host = "[::]"
allow_public_bind = false
EOF

RUN chown -R 65534:65534 /asterel-data

# ── Stage 3: Development Runtime (Debian) ────────────────────
FROM debian:bookworm-slim AS dev

# Install runtime dependencies + basic debug tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    openssl \
    curl \
    git \
    iputils-ping \
    vim \
    && rm -rf /var/lib/apt/lists/*

COPY --from=permissions /asterel-data /asterel-data
COPY --from=builder /usr/local/bin/asterel /usr/local/bin/asterel

# Keep the base config from the permissions stage for dev as well.
# Provider/model defaults are overridden by environment variables below.

# Environment setup
# Use consistent workspace path
ENV ASTEREL_WORKSPACE=/asterel-data/workspace
ENV HOME=/asterel-data
# Defaults for local dev (Ollama) - matches config.template.toml
ENV PROVIDER="ollama"
ENV ASTEREL_MODEL="llama3.2"
ENV ASTEREL_GATEWAY_PORT=3000

# Note: API_KEY is intentionally NOT set here to avoid confusion.
# It is set in config.toml as the Ollama URL.

WORKDIR /asterel-data
USER 65534:65534
EXPOSE 3000
ENTRYPOINT ["asterel"]
CMD ["--help"]

# ── Stage 4: Production Runtime (Distroless) ─────────────────
FROM gcr.io/distroless/cc-debian12:nonroot AS release

COPY --from=builder /usr/local/bin/asterel /usr/local/bin/asterel
COPY --from=permissions /asterel-data /asterel-data

# Environment setup
ENV ASTEREL_WORKSPACE=/asterel-data/workspace
ENV HOME=/asterel-data
# Defaults for prod (OpenRouter)
ENV PROVIDER="openrouter"
ENV ASTEREL_MODEL="anthropic/claude-sonnet-4-20250514"
ENV ASTEREL_GATEWAY_PORT=3000

# API_KEY must be provided at runtime!

WORKDIR /asterel-data
USER 65534:65534
EXPOSE 3000
ENTRYPOINT ["asterel"]
CMD ["--help"]
