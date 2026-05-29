# Stage 1: Build
FROM rust:1.80-slim-bookworm AS builder

# Install build dependencies (required for SQLite, crypto, fastembed, etc.)
RUN apt-get update && apt-get install -y pkg-config libssl-dev build-essential cmake clang libclang-dev

WORKDIR /app
COPY . .

# Build both the server and the TUI binaries
RUN cargo build --release --bin cade-server --bin cade

# Stage 2: Runtime
FROM debian:bookworm-slim

# Install runtime dependencies and create a non-root runtime user.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --home-dir /home/cade cade

WORKDIR /workspace

# Copy the compiled binaries from the builder stage
COPY --from=builder /app/target/release/cade-server /usr/local/bin/cade-server
COPY --from=builder /app/target/release/cade /usr/local/bin/cade

# Expose the server port
EXPOSE 8284

ENV RUST_LOG=info

USER cade

# Default command runs the server
CMD ["cade-server"]
