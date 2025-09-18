# Build stage
FROM rustlang/rust:nightly AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY felaas-oss ./felaas-oss
COPY guardianito-oss ./guardianito-oss
COPY felaas-oss-integration-tests ./felaas-oss-integration-tests

# Build the felaas-oss binary
RUN cargo build --release --bin felaas-oss

# Runtime stage
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /app/target/release/felaas-oss /usr/local/bin/felaas-oss

# Make it executable
RUN chmod +x /usr/local/bin/felaas-oss

# Default command
CMD ["/usr/local/bin/felaas-oss"]