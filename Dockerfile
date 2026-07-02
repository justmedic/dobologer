# Build stage
FROM rust:1-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/dobologer /usr/local/bin/dobologer

ENV DOBOLOGER_DATA_DIR=/data \
    DOBOLOGER_BIND_ADDR=0.0.0.0:8080 \
    DOBOLOGER_TCP_ADDR=0.0.0.0:8081 \
    DOBOLOGER_UDP_ADDR=0.0.0.0:8082

VOLUME /data

EXPOSE 8080 8081 8082/udp

ENTRYPOINT ["dobologer"]
