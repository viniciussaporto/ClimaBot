# ─── Stage 1: builder ─────────────────────────────────────────
FROM rust:1.79-slim-bookworm AS builder

WORKDIR /usr/src/climabot

# Rust is already installed in this base image; no need for curl/rustup.
# Cache dependencies separately from application code
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Compile the actual application
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ─── Stage 2: runtime ─────────────────────────────────────────
FROM debian:bookworm-slim

# Install CA certificates, gnupg, and curl (needed to add Mongo repo)
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl gnupg && \
    # Add official MongoDB 7.0 repository keys
    curl -fsSL https://www.mongodb.org/static/pgp/server-7.0.asc | \
    gpg -o /usr/share/keyrings/mongodb-server-7.0.gpg --dearmor && \
    echo "deb [ signed-by=/usr/share/keyrings/mongodb-server-7.0.gpg ] http://repo.mongodb.org/apt/debian bookworm/mongodb-org/7.0 main" | \
    tee /etc/apt/sources.list.d/mongodb-org-7.0.list && \
    apt-get update && \
    # Install MongoDB
    apt-get install -y mongodb-org && \
    # Clean up apt cache to keep image size down
    rm -rf /var/lib/apt/lists/* && \
    # Create necessary directories for Mongo and Climabot
    mkdir -p /var/log/climabot /data/db

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/climabot/target/release/climabot /usr/local/bin/climabot

# Create an entrypoint script to start MongoDB in the background, then start the bot
# This replaces the impossible `systemctl` approach.
RUN echo '#!/bin/bash\n\
# Start MongoDB as a background daemon\n\
mongod --fork --logpath /var/log/mongodb.log\n\
# Execute the Rust application\n\
exec climabot' > /usr/local/bin/start.sh && \
    chmod +x /usr/local/bin/start.sh

# Persist both bot logs and database data
VOLUME ["/var/log/climabot", "/data/db"]

# Expose Prometheus metrics and MongoDB default port
EXPOSE 9464 27017

# Run the wrapper script
CMD ["/usr/local/bin/start.sh"]