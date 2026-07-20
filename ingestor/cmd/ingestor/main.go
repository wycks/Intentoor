package main

import (
	"context"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	"intent.market/ingestor/internal/logx"
	"intent.market/ingestor/internal/providers"
	"intent.market/ingestor/internal/pub"
)

func main() {
	var (
		bindAddr     = flag.String("bind", "tcp://0.0.0.0:5555", "ZeroMQ PUB bind address")
		network      = flag.String("network", "ethereum-mainnet", "Network label")
		pollInterval = flag.Duration("poll-interval", 3*time.Second, "Polling interval")
		outDir       = flag.String("out", "./out", "Directory for JSONL logs")
	)
	flag.Parse()

	logger := log.New(os.Stdout, "ingestor: ", log.LstdFlags|log.Lmicroseconds)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	// Handle SIGINT/SIGTERM
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-sigCh
		logger.Println("shutting down...")
		cancel()
	}()

	jsonl, err := logx.NewJSONLWriter(*outDir)
	if err != nil {
		logger.Fatalf("log init: %v", err)
	}
	defer jsonl.Close()

	publisher, err := pub.NewZMQPublisher(*bindAddr)
	if err != nil {
		logger.Fatalf("zmq publisher init: %v", err)
	}
	defer publisher.Close()

	prov := []providers.Provider{
		providers.NewUniswapX(*network),
		providers.NewCoWSwap(*network),
	}

	runner := providers.NewRunner(logger, prov, publisher, jsonl, *pollInterval)
	if err := runner.Run(ctx); err != nil {
		logger.Fatalf("runner: %v", err)
	}
}
