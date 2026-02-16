# ─── Stage 1: builder ─────────────────────────────────────────
# Rust 1.85 is the first stable version to support Edition 2024
FROM rust:1.85-slim-bookworm AS builder

WORKDIR /usr/src/climabot

RUN rustc --version && cargo --version

# Build-time deps
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# ── Dependency caching layer ──────────────────────────────────
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

COPY --from=builder /usr/src/climabot/target/release/climabot /usr/local/bin/climabot

VOLUME ["/var/log/climabot"]
EXPOSE 9464

CMD ["climabot"]