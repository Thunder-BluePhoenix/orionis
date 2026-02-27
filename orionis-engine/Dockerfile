# Build stage
FROM rust:1.75-slim-bookworm as builder

WORKDIR /usr/src/orionis

# Install dependencies (including protoc and required libraries)
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    &> /dev/null

# Copy source code (from root context since proto is outside engine dir)
COPY . .

# Build the engine
WORKDIR /usr/src/orionis/orionis-engine
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    &> /dev/null \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/orionis/orionis-engine/target/release/orionis-engine /app/orionis-engine

# Expose ports
# HTTP API & Dashboard
EXPOSE 9090
# gRPC Ingestion & Clustering
EXPOSE 50051

# Default environment variables
ENV ORIONIS_ENFORCE_AUTH=true
ENV ORIONIS_JWT_SECRET=super-secret-production-key
ENV ORIONIS_API_KEY=orionis-agent-key

CMD ["/app/orionis-engine"]
