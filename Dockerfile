# Multi-stage production build for Hephos services
# Uses distroless container for minimal attack surface and zero extraneous packages.

FROM rust:1.85-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY examples/ examples/

# Build release artifacts
RUN cargo build --release -p example-reference-app

# Runtime image: Google Distroless (non-root, security-hardened)
FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

# Copy compiled binary from builder
COPY --from=builder /app/target/release/example-reference-app /app/server

# Expose standard Hephos HTTP port
EXPOSE 8080

# Configure production environment defaults
ENV HEPHOS_ADDR=0.0.0.0:8080 \
    HEPHOS_LOG=info \
    HEPHOS_LOG_FORMAT=json

# Run as non-root user (distroless nonroot UID is 65532)
USER nonroot:nonroot

ENTRYPOINT ["/app/server"]
