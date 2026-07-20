package providers

import (
	"crypto/sha256"
	"encoding/hex"
	"sync"
	"time"
)

type deduper struct {
	mu   sync.Mutex
	seen map[string]time.Time
	ttl  time.Duration
}

func newDeduper(ttl time.Duration) *deduper {
	return &deduper{seen: make(map[string]time.Time), ttl: ttl}
}

// SeenOrRemember returns true if the key is new (and remembers it), false if already seen.
func (d *deduper) SeenOrRemember(key string) bool {
	d.mu.Lock()
	defer d.mu.Unlock()

	now := time.Now()

	// opportunistic cleanup
	for k, t := range d.seen {
		if now.Sub(t) > d.ttl {
			delete(d.seen, k)
		}
	}

	if _, ok := d.seen[key]; ok {
		return false
	}
	d.seen[key] = now
	return true
}

func hashBytes(b []byte) string {
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])
}
