# ── Build Stage ───────────────────────────────────────────────────────────────
# sf-api 0.3 needs Rust >= 1.85 (edition 2024)
FROM rust:latest AS builder

WORKDIR /build
COPY Cargo.toml Cargo.toml
COPY src/ src/

# Build release binary
RUN cargo build --release 2>&1

# ── Runtime Stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/target/release/sfguild-scanner /app/sfguild-scanner

# Copy static web files
COPY static/ /app/static/

# Create data directory for persistent scan results
RUN mkdir -p /app/data

# Version label for Unraid update detection (passed via --build-arg from CI)
ARG VERSION=dev
LABEL version="${VERSION}"

EXPOSE 8080

VOLUME ["/app/data"]

ENV RUST_LOG=info

CMD ["/app/sfguild-scanner"]
