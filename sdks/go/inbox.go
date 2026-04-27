package chakramcp

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"sync"
	"time"
)

type InboxClient struct {
	c *Client
}

// Pull atomically claims the oldest pending invocations targeting an
// agent you own. Concurrent pullers (across machines) get disjoint
// batches via FOR UPDATE SKIP LOCKED at the DB.
func (in *InboxClient) Pull(ctx context.Context, agentID string, limit int) ([]Invocation, error) {
	q := url.Values{}
	q.Set("agent_id", agentID)
	if limit > 0 {
		q.Set("limit", strconv.Itoa(limit))
	}
	var out []Invocation
	if err := in.c.do(ctx, http.MethodGet, in.c.relayURL, "/v1/inbox?"+q.Encode(), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Respond reports the result for an in_progress invocation.
func (in *InboxClient) Respond(ctx context.Context, invocationID string, result HandlerResult) (*Invocation, error) {
	var out Invocation
	if err := in.c.do(ctx, http.MethodPost, in.c.relayURL, "/v1/invocations/"+url.PathEscape(invocationID)+"/result", result, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Handler is the user-supplied function dispatched per invocation by
// Serve. Return Succeeded(value) or Failed("reason") (or set the
// HandlerResult fields directly). Returning an error reports the
// invocation as failed with the error's message and the loop keeps
// going.
type Handler func(ctx context.Context, inv Invocation) (HandlerResult, error)

// ServeOptions configures Serve. Zero-valued fields fall back to
// PollInterval=2s, BatchSize=25.
type ServeOptions struct {
	PollInterval time.Duration
	BatchSize    int
	// OnError, if set, is called when a pull or respond fails.
	// It is also called when a handler returns an error.
	OnError func(err error, inv *Invocation)
}

// Serve runs a long-lived pull → handler → respond loop. It returns
// when ctx is cancelled. Handler errors are caught and reported as
// failed invocations; the loop keeps going.
func (in *InboxClient) Serve(ctx context.Context, agentID string, handler Handler, opts ServeOptions) error {
	if handler == nil {
		return errors.New("chakramcp: Serve requires a non-nil handler")
	}
	interval := opts.PollInterval
	if interval == 0 {
		interval = 2 * time.Second
	}
	batch := opts.BatchSize
	if batch == 0 {
		batch = 25
	}
	for {
		if err := ctx.Err(); err != nil {
			return nil
		}
		invs, err := in.Pull(ctx, agentID, batch)
		if err != nil {
			if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
				return nil
			}
			if opts.OnError != nil {
				opts.OnError(err, nil)
			}
			if !sleep(ctx, interval) {
				return nil
			}
			continue
		}
		if len(invs) == 0 {
			if !sleep(ctx, interval) {
				return nil
			}
			continue
		}
		// Process in parallel — invocations are independent.
		var wg sync.WaitGroup
		for _, inv := range invs {
			wg.Add(1)
			invCopy := inv
			go func() {
				defer wg.Done()
				in.handleOne(ctx, invCopy, handler, opts)
			}()
		}
		wg.Wait()
	}
}

func (in *InboxClient) handleOne(ctx context.Context, inv Invocation, handler Handler, opts ServeOptions) {
	defer func() {
		if r := recover(); r != nil {
			err := fmt.Errorf("handler panicked: %v", r)
			if opts.OnError != nil {
				opts.OnError(err, &inv)
			}
			_, _ = in.Respond(ctx, inv.ID, Failed(err.Error()))
		}
	}()
	res, err := handler(ctx, inv)
	if err != nil {
		if opts.OnError != nil {
			opts.OnError(err, &inv)
		}
		_, _ = in.Respond(ctx, inv.ID, Failed(err.Error()))
		return
	}
	if _, err := in.Respond(ctx, inv.ID, res); err != nil {
		if opts.OnError != nil {
			opts.OnError(err, &inv)
		}
	}
}

// sleep returns false if ctx was cancelled while sleeping.
func sleep(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}
