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
	"time"
)

// CallsClient covers calls: two agents talking over the relay, with
// messages, capabilities invoked in the call (InvokeRequest.CallID),
// and, in call-memory mode, a memory the relay keeps.
//
// Methods that act in a call take agentID, the party acting, and need
// an API key, OAuth or device token: an interactive web session is
// refused with chk.calls.owner_session.
type CallsClient struct {
	c *Client
}

func callPath(callID, suffix string) string {
	return "/v1/calls/" + url.PathEscape(callID) + suffix
}

// Start invites another agent to a call. The call rings until the
// callee accepts or declines it, or for 24 h, after which it's missed.
func (cs *CallsClient) Start(ctx context.Context, req *StartCallRequest) (*Call, error) {
	var out Call
	if err := cs.c.do(ctx, http.MethodPost, cs.c.relayURL, "/v1/calls", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// ListCallsOptions filters List. Zero values are left out.
type ListCallsOptions struct {
	AgentID string    // one of your agents; empty means all of them
	State   CallState // optional
	Limit   int
	Cursor  string // CallList.NextCursor of the previous page
}

// List returns your agents' calls, newest first, a page at a time.
func (cs *CallsClient) List(ctx context.Context, opts ListCallsOptions) (*CallList, error) {
	q := url.Values{}
	if opts.AgentID != "" {
		q.Set("agent_id", opts.AgentID)
	}
	if opts.State != "" {
		q.Set("state", string(opts.State))
	}
	if opts.Limit > 0 {
		q.Set("limit", strconv.Itoa(opts.Limit))
	}
	if opts.Cursor != "" {
		q.Set("cursor", opts.Cursor)
	}
	path := "/v1/calls"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out CallList
	if err := cs.c.do(ctx, http.MethodGet, cs.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Get returns one call.
func (cs *CallsClient) Get(ctx context.Context, callID string) (*Call, error) {
	var out Call
	if err := cs.c.do(ctx, http.MethodGet, cs.c.relayURL, callPath(callID, ""), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

type acceptCallBody struct {
	AgentID string `json:"agent_id"`
	Message string `json:"message,omitempty"`
}

type declineCallBody struct {
	AgentID string `json:"agent_id"`
	Reason  string `json:"reason,omitempty"`
}

type endCallBody struct {
	AgentID string `json:"agent_id"`
}

type sendCallBody struct {
	AgentID string          `json:"agent_id"`
	Text    string          `json:"text"`
	Data    json.RawMessage `json:"data,omitempty"`
}

type askCallBody struct {
	AgentID  string `json:"agent_id"`
	Question string `json:"question"`
}

// Accept answers a ringing call as agentID, the callee, with an
// optional first reply (empty for none). The call becomes active.
func (cs *CallsClient) Accept(ctx context.Context, callID, agentID, message string) (*Call, error) {
	var out Call
	body := &acceptCallBody{AgentID: agentID, Message: message}
	if err := cs.c.do(ctx, http.MethodPost, cs.c.relayURL, callPath(callID, "/accept"), body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Decline refuses a ringing call as agentID, the callee, with an
// optional reason (empty for none).
func (cs *CallsClient) Decline(ctx context.Context, callID, agentID, reason string) (*Call, error) {
	var out Call
	body := &declineCallBody{AgentID: agentID, Reason: reason}
	if err := cs.c.do(ctx, http.MethodPost, cs.c.relayURL, callPath(callID, "/decline"), body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// End ends a ringing or active call as agentID, either party.
func (cs *CallsClient) End(ctx context.Context, callID, agentID string) (*Call, error) {
	var out Call
	if err := cs.c.do(ctx, http.MethodPost, cs.c.relayURL, callPath(callID, "/end"), &endCallBody{AgentID: agentID}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Send posts a message on an active call as agentID and returns its
// transcript entry. A call that isn't active answers 409
// chk.calls.not_active.
func (cs *CallsClient) Send(ctx context.Context, callID, agentID string, msg CallMessage) (*CallEntry, error) {
	var out CallEntry
	body := &sendCallBody{AgentID: agentID, Text: msg.Text, Data: msg.Data}
	if err := cs.c.do(ctx, http.MethodPost, cs.c.relayURL, callPath(callID, "/messages"), body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// CallHistoryOptions filters History. Zero values are left out.
type CallHistoryOptions struct {
	After int64  // entries after this seq
	Limit int    // the relay's default is 50, at most 200
	Q     string // full-text search: any of the words, best match first
}

// History returns a call's stored transcript, oldest first (best match
// first with Q), until its content is deleted.
func (cs *CallsClient) History(ctx context.Context, callID string, opts CallHistoryOptions) ([]CallEntry, error) {
	q := url.Values{}
	if opts.After > 0 {
		q.Set("after", strconv.FormatInt(opts.After, 10))
	}
	if opts.Limit > 0 {
		q.Set("limit", strconv.Itoa(opts.Limit))
	}
	if opts.Q != "" {
		q.Set("q", opts.Q)
	}
	path := callPath(callID, "/messages")
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out []CallEntry
	if err := cs.c.do(ctx, http.MethodGet, cs.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Memory returns a call's current memory (call-memory mode).
func (cs *CallsClient) Memory(ctx context.Context, callID string) (*CallMemory, error) {
	var out CallMemory
	if err := cs.c.do(ctx, http.MethodGet, cs.c.relayURL, callPath(callID, "/memory"), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Ask searches a call's memory for the messages that answer question,
// as agentID, a party. Without memory features it answers 503
// chk.calls.memory_unavailable.
func (cs *CallsClient) Ask(ctx context.Context, callID, agentID, question string) (*CallRecall, error) {
	var out CallRecall
	body := &askCallBody{AgentID: agentID, Question: question}
	if err := cs.c.do(ctx, http.MethodPost, cs.c.relayURL, callPath(callID, "/memory/ask"), body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Pull claims agentID's undelivered call entries, oldest first: invites,
// messages and events. Each is delivered once. The relay returns at
// most 25; limit <= 0 takes that. Use PullWithOptions to wait for
// entries.
func (cs *CallsClient) Pull(ctx context.Context, agentID string, limit int) ([]CallFeedItem, error) {
	return cs.PullWithOptions(ctx, agentID, CallsPullOptions{Limit: limit})
}

// CallsPullOptions configures Calls().PullWithOptions and PullWithMeta.
// Zero values are left out.
type CallsPullOptions struct {
	// Limit is the most entries to claim; 0 takes the relay's default.
	Limit int
	// Wait is how many seconds the relay may hold the pull open until an
	// entry arrives (0-25; larger values are clamped). 0 doesn't wait.
	// Relays before 0.7 ignore it and answer at once.
	Wait int
}

// CallsPullResult is what Calls().PullWithMeta returns.
type CallsPullResult struct {
	Items []CallFeedItem
	// Waited is the seconds the relay honoured of CallsPullOptions.Wait,
	// as in PullResult.Waited: 0 means it didn't wait, nil means a relay
	// before 0.7 (or a header that isn't an integer).
	Waited *int
}

// PullWithOptions is Pull with options: it can wait for entries.
func (cs *CallsClient) PullWithOptions(ctx context.Context, agentID string, opts CallsPullOptions) ([]CallFeedItem, error) {
	res, err := cs.PullWithMeta(ctx, agentID, opts)
	if err != nil {
		return nil, err
	}
	return res.Items, nil
}

// PullWithMeta is PullWithOptions that also says whether the relay
// waited, for agents that run their own loop: after an empty batch,
// pull again straight away when Waited is above 0, and sleep first
// otherwise.
func (cs *CallsClient) PullWithMeta(ctx context.Context, agentID string, opts CallsPullOptions) (*CallsPullResult, error) {
	q := url.Values{}
	q.Set("agent_id", agentID)
	if opts.Limit > 0 {
		q.Set("limit", strconv.Itoa(opts.Limit))
	}
	if opts.Wait > 0 {
		q.Set("wait", strconv.Itoa(opts.Wait))
	}
	var out []CallFeedItem
	h, err := cs.c.doWithHeaders(ctx, http.MethodGet, cs.c.relayURL, "/v1/inbox/calls?"+q.Encode(), nil, &out)
	if err != nil {
		return nil, err
	}
	return &CallsPullResult{Items: out, Waited: waitedFrom(h)}, nil
}

// ─── Serve ───────────────────────────────────────────────

// CallAnswer is what a CallInviteHandler decides. The zero value
// accepts without a first reply.
type CallAnswer struct {
	// Decline refuses the call; false accepts it.
	Decline bool
	// Message is the first reply when accepting (empty for none).
	Message string
	// Reason is given when declining (empty for none).
	Reason string
}

// AcceptCall accepts an invite, with an optional first reply.
func AcceptCall(message string) CallAnswer { return CallAnswer{Message: message} }

// DeclineCall declines an invite, with an optional reason.
func DeclineCall(reason string) CallAnswer { return CallAnswer{Decline: true, Reason: reason} }

// CallInviteHandler decides on an invite (item.Entry.Kind ==
// CallEntryInvite). On an error the call is left ringing.
type CallInviteHandler func(ctx context.Context, item CallFeedItem) (CallAnswer, error)

// CallMessageHandler handles a message from the other party. Return a
// reply to send it as the serving agent, or nil for no reply.
// item.Memory carries the call's memory in call-memory mode.
type CallMessageHandler func(ctx context.Context, item CallFeedItem) (*CallMessage, error)

// CallEventHandler is told of an accepted, declined, ended or missed
// call (item.Entry.Kind).
type CallEventHandler func(ctx context.Context, item CallFeedItem) error

// CallsServeOptions configures Calls().Serve. Zero-valued fields fall
// back to WaitSecs=25, PollInterval=2s, BatchSize=25.
type CallsServeOptions struct {
	// OnMessage handles each message. Required.
	OnMessage CallMessageHandler
	// OnInvite decides on invites. Without one, every invite is
	// accepted.
	OnInvite CallInviteHandler
	// OnEvent is told of accepted, declined, ended and missed calls.
	// Without one, they're ignored.
	OnEvent CallEventHandler
	// WaitSecs is how long each pull asks the relay to wait for entries,
	// in seconds (see CallsPullOptions.Wait); 0 takes 25, and negative
	// never waits.
	WaitSecs int
	// PollInterval is the sleep after a failed pull, and after an empty
	// one the relay didn't wait for (relays before 0.7 never wait).
	PollInterval time.Duration
	BatchSize    int
	// OnError is called when a pull, an answer or a reply fails, and
	// when a handler returns an error or panics. item is nil for a
	// failed pull. Default: log it.
	OnError func(err error, item *CallFeedItem)
}

// Serve runs a pull agent's calls loop: it claims agentID's calls feed,
// answers invites, hands messages to OnMessage and sends its replies
// as agentID, and passes the other events to OnEvent, one item at a
// time in order. Each pull waits up to WaitSecs for entries; after an
// empty batch Serve pulls again at once if the relay waited, and sleeps
// PollInterval if it didn't (relays before 0.7). An answer or reply to
// a call that ended meanwhile is dropped. A handler error or panic goes
// to OnError and the loop keeps going. Serve returns nil when ctx is
// cancelled, which also aborts a pull in flight.
func (cs *CallsClient) Serve(ctx context.Context, agentID string, opts CallsServeOptions) error {
	if opts.OnMessage == nil {
		return errors.New("chakramcp: Calls().Serve requires OnMessage")
	}
	interval := opts.PollInterval
	if interval <= 0 {
		interval = 2 * time.Second
	}
	batch := opts.BatchSize
	if batch <= 0 {
		batch = 25
	}
	wait := serveWait(opts.WaitSecs)
	for {
		if ctx.Err() != nil {
			return nil
		}
		res, err := cs.PullWithMeta(ctx, agentID, CallsPullOptions{Limit: batch, Wait: wait})
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			reportCallError(opts, err, nil)
			if !sleep(ctx, interval) {
				return nil
			}
			continue
		}
		if len(res.Items) == 0 {
			if relayWaited(res.Waited) {
				continue
			}
			if !sleep(ctx, interval) {
				return nil
			}
			continue
		}
		for i := range res.Items {
			if ctx.Err() != nil {
				return nil
			}
			cs.handleItem(ctx, agentID, res.Items[i], opts)
		}
	}
}

func (cs *CallsClient) handleItem(ctx context.Context, agentID string, item CallFeedItem, opts CallsServeOptions) {
	defer func() {
		if r := recover(); r != nil {
			reportCallError(opts, fmt.Errorf("chakramcp: calls handler panicked: %v", r), &item)
		}
	}()
	switch item.Entry.Kind {
	case CallEntryInvite:
		var answer CallAnswer // accept
		if opts.OnInvite != nil {
			a, err := opts.OnInvite(ctx, item)
			if err != nil {
				reportCallError(opts, err, &item)
				return
			}
			answer = a
		}
		var err error
		if answer.Decline {
			_, err = cs.Decline(ctx, item.Call.ID, agentID, answer.Reason)
		} else {
			_, err = cs.Accept(ctx, item.Call.ID, agentID, answer.Message)
		}
		// isCallNotActive: the caller hung up, or it was missed, meanwhile.
		if err != nil && !isCallNotActive(err) {
			reportCallError(opts, err, &item)
		}
	case CallEntryMessage:
		reply, err := opts.OnMessage(ctx, item)
		if err != nil {
			reportCallError(opts, err, &item)
			return
		}
		if reply == nil || (reply.Text == "" && len(reply.Data) == 0) {
			return
		}
		if _, err := cs.Send(ctx, item.Call.ID, agentID, *reply); err != nil && !isCallNotActive(err) {
			reportCallError(opts, err, &item)
		}
	case CallEntryAccepted, CallEntryDeclined, CallEntryEnded, CallEntryMissed:
		if opts.OnEvent != nil {
			if err := opts.OnEvent(ctx, item); err != nil {
				reportCallError(opts, err, &item)
			}
		}
	}
}

// isCallNotActive: the call ended (or was never answered) meanwhile.
func isCallNotActive(err error) bool {
	var apiErr *Error
	return errors.As(err, &apiErr) && apiErr.Code == "chk.calls.not_active"
}

func reportCallError(opts CallsServeOptions, err error, item *CallFeedItem) {
	if opts.OnError != nil {
		opts.OnError(err, item)
		return
	}
	if item == nil {
		log.Printf("[chakramcp] calls: %v", err)
		return
	}
	log.Printf("[chakramcp] calls: %s %d on call %s: %v", item.Entry.Kind, item.Entry.Seq, item.Call.ID, err)
}
