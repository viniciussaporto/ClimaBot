# ─── Stage 1: builder ─────────────────────────────────────────
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /usr/src/climabot

# Build-time deps needed to compile openssl-sys and mongodb TLS
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# ── Dependency caching layer ──────────────────────────────────
# Using wildcard Cargo.lock* ensures it builds even if you deleted the lockfile
COPY Cargo.toml Cargo.lock* ./

# Dummy build to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# ── Application build ─────────────────────────────────────────
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ─── Stage 2: runtime ─────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/* && \
    mkdir -p /var/log/climabot

# Ensure we copy the binary from the correct path
COPY --from=builder /usr/src/climabot/target/release/climabot /usr/local/bin/climabot

VOLUME ["/var/log/climabot"]

# Prometheus metrics
EXPOSE 9464

CMD ["climabot"]