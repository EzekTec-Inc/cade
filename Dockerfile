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

# Install runtime dependencies
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy the compiled binaries from the builder stage
COPY --from=builder /app/target/release/cade-server /usr/local/bin/cade-server
COPY --from=builder /app/target/release/cade /usr/local/bin/cade

# Expose the server port
EXPOSE 8284

ENV RUST_LOG=info

# Default command runs the server
CMD ["cade-server"]
