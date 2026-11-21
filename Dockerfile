# ─── Stage 1: builder ─────────────────────────────────────────
FROM rust:1.100-slim-bookworm AS builder

# Limit parallel rustc processes; the production host has little free RAM.
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

# OpenSSL headers for native-tls
RUN apt-get update && \
    apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/climabot

# ── Dependency caching layer ──────────────────────────────────
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --locked && \
    cargo clean --release --package climabot && \
    rm -rf src

# ── Application build ─────────────────────────────────────────
COPY images ./images
COPY src ./src
# `cargo clean -p` above forgets the placeholder build whatever Cargo's
# build-directory layout; touching main.rs makes sure the real sources are
# rebuilt even if COPY kept old timestamps. The version check fails the build
# if the placeholder binary (which prints nothing) slipped through anyway.
RUN touch src/main.rs && \
    cargo build --release --locked && \
    ./target/release/climabot --version | grep -q '^climabot '

# ─── Stage 2: runtime ─────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin climabot && \
    mkdir -p /var/log/climabot && \
    chown climabot:climabot /var/log/climabot

COPY --from=builder /usr/src/climabot/target/release/climabot /usr/local/bin/climabot

USER climabot
ENV LOG_DIR=/var/log/climabot \
    METRICS_PORT=9464

VOLUME ["/var/log/climabot"]
EXPOSE 9464

CMD ["climabot"]
