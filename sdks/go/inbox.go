package chakramcp

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
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

// RespondResult is what Respond returns: exactly one of Invocation (the
// call finished) or Drafted (a human_in_loop reply, kept as a draft
// until one of the agent's owners confirms it at Drafted.ApproveURL).
type RespondResult struct {
	Invocation *Invocation
	Drafted    *DraftedReply
}

// Respond reports the result for an in_progress invocation.
func (in *InboxClient) Respond(ctx context.Context, invocationID string, result HandlerResult) (*RespondResult, error) {
	var raw json.RawMessage
	if err := in.c.do(ctx, http.MethodPost, in.c.relayURL, "/v1/invocations/"+url.PathEscape(invocationID)+"/result", result, &raw); err != nil {
		return nil, err
	}
	var probe struct {
		Status string `json:"status"`
	}
	if err := json.Unmarshal(raw, &probe); err != nil {
		return nil, fmt.Errorf("chakramcp: decode result: %w", err)
	}
	if probe.Status == "awaiting_confirmation" {
		var d DraftedReply
		if err := json.Unmarshal(raw, &d); err != nil {
			return nil, fmt.Errorf("chakramcp: decode draft: %w", err)
		}
		return &RespondResult{Drafted: &d}, nil
	}
	var inv Invocation
	if err := json.Unmarshal(raw, &inv); err != nil {
		return nil, fmt.Errorf("chakramcp: decode invocation: %w", err)
	}
	return &RespondResult{Invocation: &inv}, nil
}

// Notices claims the agent's notices, oldest first; each is delivered
// once. A NoticeConsentRequested notice means a call waits for your
// owner's approval: pass ConsentRequest.ApproveURL on to them.
func (in *InboxClient) Notices(ctx context.Context, agentID string, limit int) ([]Notice, error) {
	q := url.Values{}
	q.Set("agent_id", agentID)
	if limit > 0 {
		q.Set("limit", strconv.Itoa(limit))
	}
	var out []Notice
	if err := in.c.do(ctx, http.MethodGet, in.c.relayURL, "/v1/inbox/notices?"+q.Encode(), nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Handler is the user-supplied function dispatched per invocation by
// Serve. Return Succeeded(value) or Failed("reason") (or set the
// HandlerResult fields directly). Returning an error reports the
// invocation as failed with the error's message and the loop keeps
// going.
type Handler func(ctx context.Context, inv Invocation) (HandlerResult, error)

// HumanHandler handles a human_in_loop invocation: a person stands
// behind each reply. Return a result to propose a reply (for example,
// one your model drafted); the relay keeps it as a draft until one of
// the agent's owners confirms it on the web. Return nil to leave the
// reply to a person, who answers in the web app's inbox; use this to
// notify them. On an error nothing is posted.
type HumanHandler func(ctx context.Context, inv Invocation) (*HandlerResult, error)

// ServeOptions configures Serve. Zero-valued fields fall back to
// PollInterval=2s, BatchSize=25, NoticeInterval=15s.
type ServeOptions struct {
	PollInterval time.Duration
	BatchSize    int
	// HumanHandler handles human_in_loop invocations. Without one, Serve
	// logs a warning and leaves them in_progress for the owner to answer
	// in the web app's inbox.
	HumanHandler HumanHandler
	// OnDraft is called when a HumanHandler reply was stored as a draft.
	// Default: log the confirmation link.
	OnDraft func(inv Invocation, drafted DraftedReply)
	// OnNotice is called for each notice: NoticeConsentRequested (a call
	// waits for your owner's approval) and what became of drafted
	// replies. Default: log consent links.
	OnNotice func(n Notice)
	// NoticeInterval is how often to claim notices; negative turns it off.
	NoticeInterval time.Duration
	// OnError, if set, is called when a pull or respond fails.
	// It is also called when a handler returns an error.
	OnError func(err error, inv *Invocation)
}

func defaultOnNotice(n Notice) {
	r := n.ConsentRequest
	if n.Kind != NoticeConsentRequested || r == nil {
		return
	}
	who, what := "An agent", "a capability"
	if r.Requester.AgentDisplayName != nil {
		who = *r.Requester.AgentDisplayName
	}
	if r.CapabilityName != nil {
		what = *r.CapabilityName
	}
	log.Printf("[chakramcp] %s asks to use %s; it waits for your owner's approval: %s", who, what, r.ApproveURL)
}

func defaultOnDraft(inv Invocation, d DraftedReply) {
	log.Printf("[chakramcp] reply to %s saved as a draft; your owner confirms it before the caller sees it: %s", inv.ID, d.ApproveURL)
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
	noticeEvery := opts.NoticeInterval
	if noticeEvery == 0 {
		noticeEvery = 15 * time.Second
	}
	var lastNotices time.Time
	for {
		if err := ctx.Err(); err != nil {
			return nil
		}
		if noticeEvery > 0 && time.Since(lastNotices) >= noticeEvery {
			lastNotices = time.Now()
			in.dispatchNotices(ctx, agentID, opts)
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
		// Process in parallel - invocations are independent.
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

func (in *InboxClient) dispatchNotices(ctx context.Context, agentID string, opts ServeOptions) {
	notices, err := in.Notices(ctx, agentID, 0)
	if err != nil {
		if opts.OnError != nil && ctx.Err() == nil {
			opts.OnError(err, nil)
		}
		return
	}
	onNotice := opts.OnNotice
	if onNotice == nil {
		onNotice = defaultOnNotice
	}
	for _, n := range notices {
		onNotice(n)
	}
}

func (in *InboxClient) handleHuman(ctx context.Context, inv Invocation, opts ServeOptions) {
	if opts.HumanHandler == nil {
		log.Printf("[chakramcp] warning: invocation %s on capability %q is human_in_loop, but no HumanHandler is set. Leaving it in_progress for your owner to answer in the web app's inbox.", inv.ID, inv.CapabilityName)
		return
	}
	// Don't post `failed` on an error or a panic: a person still has to
	// answer, and closing the call would take that away from them.
	defer func() {
		if r := recover(); r != nil && opts.OnError != nil {
			opts.OnError(fmt.Errorf("human handler panicked: %v", r), &inv)
		}
	}()
	proposed, err := opts.HumanHandler(ctx, inv)
	if err != nil {
		if opts.OnError != nil {
			opts.OnError(err, &inv)
		}
		return
	}
	if proposed == nil {
		return
	}
	res, err := in.Respond(ctx, inv.ID, *proposed)
	if err != nil {
		if opts.OnError != nil {
			opts.OnError(err, &inv)
		}
		return
	}
	if res.Drafted != nil {
		onDraft := opts.OnDraft
		if onDraft == nil {
			onDraft = defaultOnDraft
		}
		onDraft(inv, *res.Drafted)
	}
}

func (in *InboxClient) handleOne(ctx context.Context, inv Invocation, handler Handler, opts ServeOptions) {
	if inv.Semantics == SemanticsHumanInLoop {
		in.handleHuman(ctx, inv, opts)
		return
	}
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
