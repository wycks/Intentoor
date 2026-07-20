package providers

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"
)

type CoWSwap struct {
	network string
	baseURL string
	client  *http.Client
}

func NewCoWSwap(network string) *CoWSwap {
	// Per CoW Protocol OpenAPI, /api/v1/auction is permissioned on prod.
	// Barn is accessible in more environments, so default to barn unless overridden.
	baseURL := os.Getenv("COW_API_BASE_URL")
	if baseURL == "" {
		baseURL = "https://barn.api.cow.fi/mainnet"
	}
	return &CoWSwap{
		network: network,
		baseURL: baseURL,
		client:  &http.Client{Timeout: 10 * time.Second},
	}
}

func (c *CoWSwap) Name() string { return "cowswap" }

func (c *CoWSwap) Poll(ctx context.Context) ([]Envelope, error) {
	url := c.baseURL + "/api/v1/auction"

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("accept", "application/json")
	req.Header.Set("user-agent", "intent-market/0.1 (+https://local)")
	resp, err := c.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("http %d: %s", resp.StatusCode, truncate(string(body), 500))
	}

	// CoW auction response is expected to contain an orders list; flatten to one envelope per order.
	var parsed struct {
		Orders []json.RawMessage `json:"orders"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil {
		// If the shape differs, still emit one envelope so we capture payloads for later inspection.
		env := Envelope{
			SchemaVersion: "0.1.0",
			EmittedAt:     time.Now().UTC(),
			Source:        c.Name(),
			Network:       c.network,
			TransportHint: "zmq:pub",
			Raw:           json.RawMessage(body),
			Meta: map[string]any{
				"poller":       "cowswap/auction",
				"http_status":  resp.StatusCode,
				"decode_error": err.Error(),
			},
		}
		return []Envelope{env}, nil
	}

	emittedAt := time.Now().UTC()
	out := make([]Envelope, 0, len(parsed.Orders))
	for _, rawOrder := range parsed.Orders {
		id, normalized := normalizeCoW(rawOrder)
		out = append(out, Envelope{
			SchemaVersion: "0.1.0",
			EmittedAt:     emittedAt,
			Source:        c.Name(),
			Network:       c.network,
			TransportHint: "zmq:pub",
			ID:            id,
			Normalized:    normalized,
			Raw:           rawOrder,
			Meta: map[string]any{
				"poller":      "cowswap/auction",
				"http_status": resp.StatusCode,
				"api_base":    c.baseURL,
			},
		})
	}
	return out, nil
}

func normalizeCoW(raw json.RawMessage) (string, map[string]any) {
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		return "", nil
	}

	// Common CoW order identifiers include "uid".
	id, _ := m["uid"].(string)
	if id == "" {
		id, _ = m["id"].(string)
	}

	n := make(map[string]any)
	if sell, ok := m["sellToken"].(string); ok {
		n["sell_token"] = sell
	}
	if buy, ok := m["buyToken"].(string); ok {
		n["buy_token"] = buy
	}
	if sellAmt, ok := m["sellAmount"].(string); ok {
		n["sell_amount"] = sellAmt
	}
	if minBuyAmt, ok := m["buyAmount"].(string); ok {
		// Note: CoW has both buyAmount and (sometimes) buyAmount/fee adjustments.
		n["min_buy_amount"] = minBuyAmt
	}
	if validTo, ok := m["validTo"].(float64); ok {
		n["deadline_unix"] = int64(validTo)
	}

	if len(n) == 0 {
		return id, nil
	}
	return id, n
}
