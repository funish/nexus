FROM rust:slim-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY index.html ./
COPY public ./public

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/nexus /usr/local/bin/

ENV PORT=3000
EXPOSE 3000

CMD ["nexus"]
