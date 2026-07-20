package logx

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"
)

type JSONLWriter struct {
	mu     sync.Mutex
	file   *os.File
	writer *bufio.Writer
}

func NewJSONLWriter(dir string) (*JSONLWriter, error) {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	name := fmt.Sprintf("intents-%s.jsonl", time.Now().UTC().Format("20060102-150405"))
	path := filepath.Join(dir, name)

	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
	if err != nil {
		return nil, err
	}
	return &JSONLWriter{file: f, writer: bufio.NewWriterSize(f, 1024*1024)}, nil
}

func (w *JSONLWriter) WriteLine(b []byte) error {
	w.mu.Lock()
	defer w.mu.Unlock()

	if _, err := w.writer.Write(b); err != nil {
		return err
	}
	if err := w.writer.WriteByte('\n'); err != nil {
		return err
	}
	return w.writer.Flush()
}

func (w *JSONLWriter) Close() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.writer != nil {
		_ = w.writer.Flush()
	}
	if w.file != nil {
		_ = w.file.Close()
	}
}
