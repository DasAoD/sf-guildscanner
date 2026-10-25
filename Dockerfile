# ── Build Stage ───────────────────────────────────────────────────────────────
FROM rust:latest AS builder

WORKDIR /build

RUN rustup target add x86_64-unknown-linux-musl
RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*

# Abhängigkeiten in eigener Schicht mit Dummy-main bauen: Solange sich
# Cargo.toml/Cargo.lock nicht ändern, kommt diese Schicht aus dem Cache und
# bei Code-Änderungen wird nur noch der eigene Crate neu kompiliert.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs     && cargo build --release --target x86_64-unknown-linux-musl

COPY src/ src/

# Build static binary (no GLIBC dependency). touch, damit cargo die echte
# main.rs sicher als neuer als den Dummy-Build erkennt.
RUN touch src/main.rs && cargo build --release --target x86_64-unknown-linux-musl 2>&1

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

# Selbe VERSION auch zur Laufzeit verfügbar machen (nicht nur als Image-
# Label), damit der Server sie z.B. im Web-UI anzeigen kann.
ENV APP_VERSION=${VERSION}

EXPOSE 8080

VOLUME ["/app/data"]

ENV RUST_LOG=info

CMD ["/app/sfguild-scanner"]