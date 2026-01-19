# --- BUILD STAGE ---
FROM rust:1.75-slim-bookworm AS builder

# Install system dependencies for build
RUN apt-get update && apt-get install -y \
    pkg-config \
    libhwloc-dev \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/vortex
COPY . .

# Build for release
# We build both the server and the benchmark tools
RUN cargo build --release --bin vortex-server --example vortex_stress

# --- RUNTIME STAGE ---
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libhwloc15 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries from the builder stage
COPY --from=builder /usr/src/vortex/target/release/vortex-server /usr/local/bin/vortex-server
COPY --from=builder /usr/src/vortex/target/release/examples/vortex_stress /usr/local/bin/vortex_stress

# Create data directory for WAL
RUN mkdir -p /var/lib/vortex/data

# High performance defaults for container execution
EXPOSE 8080
ENTRYPOINT ["vortex-server"]
CMD ["-p", "8080", "-d", "/var/lib/vortex/data"]
