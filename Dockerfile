# ─── Stage 1: builder ─────────────────────────────────────────
FROM rust:1.93-slim-bookworm AS builder

# Limit parallel rustc processes; the production host has little free RAM.
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}

WORKDIR /usr/src/climabot

# ── Dependency caching layer ──────────────────────────────────
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release --locked && \
    rm -rf src target/release/climabot target/release/deps/climabot-*

# ── Application build ─────────────────────────────────────────
COPY images ./images
COPY src ./src
RUN cargo build --release --locked

# ─── Stage 2: runtime ─────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
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
