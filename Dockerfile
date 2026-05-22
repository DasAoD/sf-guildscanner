# ── Build Stage ───────────────────────────────────────────────────────────────
FROM rust:latest AS builder

WORKDIR /build
COPY Cargo.toml Cargo.toml
COPY src/ src/

RUN rustup target add x86_64-unknown-linux-musl
RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*

# Build static binary (no GLIBC dependency)
RUN cargo build --release --target x86_64-unknown-linux-musl 2>&1

# ── Runtime Stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy static binary from builder
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/sfguild-scanner /app/sfguild-scanner

# Copy static web files
COPY static/ /app/static/

# Create data directory for persistent scan results
RUN mkdir -p /app/data

# Version label for Unraid update detection (passed via --build-arg from CI)
ARG VERSION=dev
LABEL version="${VERSION}" \
      net.unraid.docker.webui="http://[IP]:[PORT:8085]/" \
      net.unraid.docker.icon="https://raw.githubusercontent.com/DasAoD/sf-guildscanner/main/unraid/icon.jpg" \
      net.unraid.docker.project="https://github.com/DasAoD/sf-guildscanner" \
      net.unraid.docker.support="https://github.com/DasAoD/sf-guildscanner/issues" \
      net.unraid.docker.readme="https://github.com/DasAoD/sf-guildscanner#readme" \
      org.opencontainers.image.url="https://github.com/DasAoD/sf-guildscanner" \
      org.opencontainers.image.documentation="https://github.com/DasAoD/sf-guildscanner#readme" \
      org.opencontainers.image.source="https://github.com/DasAoD/sf-guildscanner" \
      org.opencontainers.image.description="SF Gilden-Scanner – findet angreifbare Gilden in Shakes & Fidget. Rust + Axum, Web UI, Docker."

EXPOSE 8080

VOLUME ["/app/data"]

ENV RUST_LOG=info

CMD ["/app/sfguild-scanner"]