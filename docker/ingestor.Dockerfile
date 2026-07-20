# syntax=docker/dockerfile:1

FROM golang:1.22-bookworm AS build

RUN apt-get update \
  && apt-get install -y --no-install-recommends pkg-config libzmq3-dev \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /src

COPY ingestor/go.mod ingestor/go.sum ./ingestor/
WORKDIR /src/ingestor
RUN go mod download

COPY ingestor/ ./

RUN CGO_ENABLED=1 GOOS=linux GOARCH=amd64 go build -o /out/ingestor ./cmd/ingestor

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libzmq5 \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /out/ingestor /app/ingestor

# logs written to /app/out by default
EXPOSE 5555

ENTRYPOINT ["/app/ingestor"]
