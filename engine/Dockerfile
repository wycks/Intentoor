# syntax=docker/dockerfile:1

FROM rust:1.88-bookworm AS build

RUN apt-get update \
  && apt-get install -y --no-install-recommends pkg-config libzmq3-dev \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy the whole Rust workspace (context is ./engine)
COPY . .

RUN cargo build --release -p engine -p tui

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libzmq5 \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /src/target/release/engine /app/engine
COPY --from=build /src/target/release/tui /app/tui
