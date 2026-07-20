package providers

import (
	"context"
	"encoding/json"
	"time"
)

// Envelope is the message sent over ZMQ and written to JSONL.
// It is intentionally permissive: we keep the raw payload and add best-effort normalized fields.
type Envelope struct {
	SchemaVersion string          `json:"schema_version"`
	EmittedAt     time.Time       `json:"emitted_at"`
	Source        string          `json:"source"`
	Network       string          `json:"network,omitempty"`
	TransportHint string          `json:"transport_hint,omitempty"`
	ID            string          `json:"id,omitempty"`
	Normalized    map[string]any  `json:"normalized,omitempty"`
	Raw           json.RawMessage `json:"raw"`
	Meta          map[string]any  `json:"meta,omitempty"`
}

type Provider interface {
	Name() string
	Poll(ctx context.Context) ([]Envelope, error)
}
