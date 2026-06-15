# Base stage: rust toolchain + cargo-chef + build deps. Cached as a layer; rarely
# changes, so the cargo-chef install only runs once.
FROM rust:slim-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

# Planner: derive a dependency recipe from Cargo.toml/Cargo.lock. The recipe is the
# cache key for the cook step below — it only changes when dependencies change, so
# editing src never invalidates the (expensive) dependency-compile layer.
FROM chef AS planner
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

# Builder: pre-compile dependencies (cached via recipe), then build only the app.
# Dependency caching relies on docker layer cache (local) + registry cache
# (CI: cache-from/to with mode=max), not on BuildKit cache mounts — those don't
# persist across fresh CI runners.
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY index.html ./
COPY public ./public
RUN cargo build --release --bin nexus

# Runtime: minimal image, GLIBC-aligned with the bookworm builder.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/nexus /usr/local/bin/
ENV PORT=3000
EXPOSE 3000
CMD ["nexus"]
