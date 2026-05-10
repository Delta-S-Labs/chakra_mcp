# Builds the chakramcp-server supervisor binary (runs both
# chakramcp-app and chakramcp-relay in one process).
#
# Build context = repo root:
#     docker build -f infra/Dockerfile -t chakramcp-server:latest .
#
# Cross-compile from arm64 mac → linux/amd64 (Lightsail Intel):
#     docker buildx build --platform linux/amd64 -f infra/Dockerfile \
#         -t <ECR_URI>:latest --push .

ARG RUST_VERSION=1.95

# ─── Stage 1: build ───────────────────────────────────────────
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy the workspace. .sqlx/ is committed so SQLX_OFFLINE=true works
# without a live database during build.
COPY backend ./backend

ENV SQLX_OFFLINE=true

WORKDIR /build/backend
RUN cargo build --release --bin chakramcp-server --locked

# ─── Stage 2: runtime ─────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --shell /usr/sbin/nologin chakra

COPY --from=builder /build/backend/target/release/chakramcp-server \
    /usr/local/bin/chakramcp-server

USER chakra
WORKDIR /home/chakra

# 8080 = chakramcp-app (auth, signup, API keys)
# 8090 = chakramcp-relay (agents, friendships, grants, MCP, discovery)
EXPOSE 8080 8090

ENTRYPOINT ["/usr/local/bin/chakramcp-server"]
CMD ["start"]
