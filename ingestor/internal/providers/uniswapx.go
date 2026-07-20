package providers

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

type UniswapX struct {
	network string
	client  *http.Client
}

func chainIDToNetworkLabel(chainID int64, fallback string) string {
	switch chainID {
	case 1:
		return "ethereum-mainnet"
	case 8453:
		return "base-mainnet"
	default:
		return fallback
	}
}

func NewUniswapX(network string) *UniswapX {
	return &UniswapX{
		network: network,
		client:  &http.Client{Timeout: 10 * time.Second},
	}
}

func (u *UniswapX) Name() string { return "uniswapx" }

func (u *UniswapX) Poll(ctx context.Context) ([]Envelope, error) {
	// Endpoint per project instructions.
	url := "https://api.uniswap.org/v2/orders?orderStatus=open"

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("accept", "application/json")
	req.Header.Set("user-agent", "intent-market/0.1 (+https://local)")
	resp, err := u.client.Do(req)
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

	var parsed struct {
		Orders []json.RawMessage `json:"orders"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil {
		return nil, fmt.Errorf("decode: %w", err)
	}

	emittedAt := time.Now().UTC()
	out := make([]Envelope, 0, len(parsed.Orders))
	for _, rawOrder := range parsed.Orders {
		id, normalized := normalizeUniswapX(rawOrder)
		network := u.network
		if v, ok := normalized["chain_id"].(int64); ok {
			network = chainIDToNetworkLabel(v, u.network)
		}
		out = append(out, Envelope{
			SchemaVersion: "0.1.0",
			EmittedAt:     emittedAt,
			Source:        u.Name(),
			Network:       network,
			TransportHint: "zmq:pub",
			ID:            id,
			Normalized:    normalized,
			Raw:           rawOrder,
			Meta: map[string]any{
				"poller":      "uniswapx/open-orders",
				"http_status": resp.StatusCode,
			},
		})
	}
	return out, nil
}

func normalizeUniswapX(raw json.RawMessage) (string, map[string]any) {
	// Use very loose decoding; we’ll tighten once we confirm fields.
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		return "", nil
	}

	id, _ := m["orderHash"].(string)
	if id == "" {
		id, _ = m["quoteId"].(string)
	}

	n := make(map[string]any)

	// Dutch auction v2 orders expose amounts in nested startAmount/endAmount.
	if input, ok := m["input"].(map[string]any); ok {
		if token, ok := input["token"].(string); ok {
			n["sell_token"] = token
		}
		if start, ok := input["startAmount"].(string); ok {
			n["sell_amount"] = start
			n["sell_amount_start"] = start
		}
		if end, ok := input["endAmount"].(string); ok {
			n["sell_amount_end"] = end
		}
		// Priority orders use a single fixed amount.
		if amt, ok := input["amount"].(string); ok {
			if _, exists := n["sell_amount"]; !exists {
				n["sell_amount"] = amt
				n["sell_amount_start"] = amt
				n["sell_amount_end"] = amt
			}
		}
	}
	if outs, ok := m["outputs"].([]any); ok && len(outs) > 0 {
		if o0, ok := outs[0].(map[string]any); ok {
			if token, ok := o0["token"].(string); ok {
				n["buy_token"] = token
			}
			if start, ok := o0["startAmount"].(string); ok {
				n["buy_amount_start"] = start
			}
			if end, ok := o0["endAmount"].(string); ok {
				n["buy_amount_end"] = end
				// For compatibility with the shared view, treat the worst-case output
				// as the "min buy".
				n["min_buy_amount"] = end
			}
			// Priority orders use a single fixed amount.
			if amt, ok := o0["amount"].(string); ok {
				if _, exists := n["min_buy_amount"]; !exists {
					n["min_buy_amount"] = amt
					n["buy_amount_start"] = amt
					n["buy_amount_end"] = amt
				}
			}
		}
	}

	if typ, ok := m["type"].(string); ok {
		n["order_type"] = typ
	}

	if cos, ok := m["cosignerData"].(map[string]any); ok {
		if ds, ok := cos["decayStartTime"].(float64); ok {
			n["decay_start_unix"] = int64(ds)
		}
		if de, ok := cos["decayEndTime"].(float64); ok {
			n["decay_end_unix"] = int64(de)
		}
		if ex, ok := cos["exclusiveFiller"].(string); ok {
			n["exclusive_filler"] = ex
			// Best-effort: treat decay start as the end of exclusivity.
			if v, ok := n["decay_start_unix"].(int64); ok {
				n["exclusive_until_unix"] = v
			}
		}
	}

	// Best-effort: if the payload contains a deadline-ish value, capture it.
	// (UniswapX may provide different validity fields; we’ll refine after inspection.)
	if deadline, ok := m["deadline"].(float64); ok {
		n["deadline_unix"] = int64(deadline)
	}
	if createdAt, ok := m["createdAt"].(float64); ok {
		n["created_at_unix"] = int64(createdAt)
	}
	if chainID, ok := m["chainId"].(float64); ok {
		n["chain_id"] = int64(chainID)
	}

	if len(n) == 0 {
		return id, nil
	}
	return id, n
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "..."
}
