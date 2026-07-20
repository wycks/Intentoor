# syntax=docker/dockerfile:1

FROM rust:latest as build

RUN apt-get update \
  && apt-get install -y --no-install-recommends pkg-config libzmq3-dev \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Cache deps
COPY engine/Cargo.toml ./engine/Cargo.toml
COPY engine/crates/engine/Cargo.toml ./engine/crates/engine/Cargo.toml
COPY engine/crates/tui/Cargo.toml ./engine/crates/tui/Cargo.toml

RUN mkdir -p engine/crates/engine/src engine/crates/tui/src \
  && printf 'fn main(){}' > engine/crates/engine/src/main.rs \
  && printf 'fn main(){}' > engine/crates/tui/src/main.rs

WORKDIR /src/engine
RUN cargo build --release -p engine -p tui

# Build real sources
WORKDIR /src
COPY engine/ ./engine/
WORKDIR /src/engine
RUN cargo clean -p engine -p tui && cargo build --release -p engine -p tui

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libzmq5 \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /src/engine/target/release/engine /app/engine
COPY --from=build /src/engine/target/release/tui /app/tui
