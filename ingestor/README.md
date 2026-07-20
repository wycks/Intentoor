## ingestor

Polls UniswapX + CoW Swap and publishes raw payload envelopes via ZeroMQ PUB.

### Run (docker)

From repo root:

- `docker compose up --build`

### Run (mac)

Prereqs:

- `brew install zeromq`
- Go 1.22+

Commands:

- `go run ./cmd/ingestor --bind tcp://0.0.0.0:5555`

Logs:

- JSONL files written under `./out` (configurable via `--out`).
