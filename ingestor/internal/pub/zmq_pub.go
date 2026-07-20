package pub

import (
	"github.com/pebbe/zmq4"
)

type ZMQPublisher struct {
	sock *zmq4.Socket
}

func NewZMQPublisher(bindAddr string) (*ZMQPublisher, error) {
	sock, err := zmq4.NewSocket(zmq4.PUB)
	if err != nil {
		return nil, err
	}
	// High-water mark to avoid unbounded memory if a subscriber is slow.
	_ = sock.SetSndhwm(10_000)
	if err := sock.Bind(bindAddr); err != nil {
		_ = sock.Close()
		return nil, err
	}
	return &ZMQPublisher{sock: sock}, nil
}

func (p *ZMQPublisher) Publish(msg []byte) error {
	_, err := p.sock.SendBytes(msg, 0)
	return err
}

func (p *ZMQPublisher) Close() {
	if p.sock != nil {
		_ = p.sock.Close()
	}
}
