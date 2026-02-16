# ─── Stage 1: builder ─────────────────────────────────────────
# rust:1.83 ships with Cargo 1.83, which supports edition2024.
# 1.79 was too old and caused: "feature `edition2024` is required"
FROM rustlang/rust:nightly-slim-bookworm AS builder

WORKDIR /usr/src/climabot

# Build-time deps needed to compile openssl-sys and mongodb TLS
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# ── Dependency caching layer ──────────────────────────────────
# Copy only the manifest first so Docker reuses this layer whenever
# Cargo.toml / Cargo.lock are unchanged.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# ── Application build ─────────────────────────────────────────
COPY src ./src
# Touch main.rs so cargo sees the real source as newer than the cached dummy
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

# Prometheus metrics
EXPOSE 9464

CMD ["climabot"]