package providers

import (
	"context"
	"encoding/json"
	"log"
	"time"

	"intent.market/ingestor/internal/logx"
	"intent.market/ingestor/internal/pub"
)

type Runner struct {
	logger       *log.Logger
	providers    []Provider
	publisher    *pub.ZMQPublisher
	jsonl        *logx.JSONLWriter
	pollInterval time.Duration
}

func NewRunner(logger *log.Logger, providers []Provider, publisher *pub.ZMQPublisher, jsonl *logx.JSONLWriter, pollInterval time.Duration) *Runner {
	return &Runner{
		logger:       logger,
		providers:    providers,
		publisher:    publisher,
		jsonl:        jsonl,
		pollInterval: pollInterval,
	}
}

func (r *Runner) Run(ctx context.Context) error {
	ticker := time.NewTicker(r.pollInterval)
	defer ticker.Stop()

	dedup := newDeduper(30 * time.Minute)

	r.logger.Printf("starting: providers=%d interval=%s", len(r.providers), r.pollInterval)

	for {
		if err := r.pollOnce(ctx, dedup); err != nil {
			r.logger.Printf("poll error: %v", err)
		}

		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			continue
		}
	}
}

func (r *Runner) pollOnce(ctx context.Context, dedup *deduper) error {
	for _, p := range r.providers {
		envs, err := p.Poll(ctx)
		if err != nil {
			r.logger.Printf("provider %s: %v", p.Name(), err)
			continue
		}
		for _, env := range envs {
			// Best-effort dedup: prefer env.ID, else hash raw.
			key := env.ID
			if key == "" {
				key = hashBytes(env.Raw)
			}
			isNew := dedup.SeenOrRemember(key)

			b, err := json.Marshal(env)
			if err != nil {
				r.logger.Printf("marshal envelope: %v", err)
				continue
			}

			// Write only new envelopes to disk to avoid unbounded growth,
			// but always publish so late subscribers (engine/tui) still see
			// the current active set.
			if isNew {
				if err := r.jsonl.WriteLine(b); err != nil {
					r.logger.Printf("jsonl write: %v", err)
				}
			}
			if err := r.publisher.Publish(b); err != nil {
				r.logger.Printf("zmq publish: %v", err)
			}
		}
	}
	return nil
}
