package chakramcp

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// callJSON is a call as the relay sends it.
func callJSON(id, state string) string {
	return `{"id":"` + id + `","state":"` + state + `",` +
		`"caller":{"agent_id":"a1","account_id":"acc1","display_name":"Scheduler","account_slug":"alice"},` +
		`"callee":{"agent_id":"a2","account_id":"acc2","display_name":"Booker","account_slug":"bob"},` +
		`"memory":"call","memory_features":{"tags":true,"recall":true},` +
		`"ttl_secs":0,"idle_secs":86400,"topic":"Q4 offsite",` +
		`"created_at":"2026-10-26T10:00:00Z","answered_at":null,"ended_at":null,"ended_by":null,` +
		`"purge_at":null,"purged_at":null,"message_count":0,"last_message_at":null}`
}

// readBody decodes a request's JSON body into a map.
func readBody(t *testing.T, r *http.Request) map[string]any {
	t.Helper()
	raw, err := io.ReadAll(r.Body)
	if err != nil {
		t.Errorf("read body: %v", err)
		return nil
	}
	var body map[string]any
	if err := json.Unmarshal(raw, &body); err != nil {
		t.Errorf("decode body %q: %v", raw, err)
	}
	return body
}

func TestCallsStartSendsBodyAndParsesCall(t *testing.T) {
	var body map[string]any
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/v1/calls" {
			http.NotFound(w, r)
			return
		}
		body = readBody(t, r)
		w.WriteHeader(http.StatusCreated)
		_, _ = w.Write([]byte(callJSON("c1", "ringing")))
	})
	ttl := int64(0)
	call, err := c.Calls().Start(context.Background(), &StartCallRequest{
		FromAgentID: "a1",
		ToAgentID:   "a2",
		Memory:      CallMemoryCall,
		TTLSecs:     &ttl,
		Topic:       "Q4 offsite",
		Message:     "Can we book the offsite?",
	})
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	want := map[string]any{
		"from_agent_id": "a1",
		"to_agent_id":   "a2",
		"memory":        "call",
		"ttl_secs":      float64(0),
		"topic":         "Q4 offsite",
		"message":       "Can we book the offsite?",
	}
	for k, v := range want {
		if body[k] != v {
			t.Errorf("body[%q] = %v, want %v", k, body[k], v)
		}
	}
	if _, ok := body["idle_secs"]; ok {
		t.Errorf("idle_secs sent though unset: %v", body)
	}
	if call.ID != "c1" || call.State != CallRinging || call.Memory != CallMemoryCall {
		t.Errorf("call = %+v", call)
	}
	if !call.MemoryFeatures.Tags || !call.MemoryFeatures.Recall {
		t.Errorf("memory_features = %+v", call.MemoryFeatures)
	}
	if call.Caller.AgentID == nil || *call.Caller.AgentID != "a1" || call.Callee.AccountID != "acc2" {
		t.Errorf("parties = %+v / %+v", call.Caller, call.Callee)
	}
	if call.Topic == nil || *call.Topic != "Q4 offsite" || call.IdleSecs != 86400 || call.TTLSecs != 0 {
		t.Errorf("terms = %+v", call)
	}
	if call.AnsweredAt != nil || call.EndedBy != nil {
		t.Errorf("answered_at/ended_by = %v/%v, want nil", call.AnsweredAt, call.EndedBy)
	}
}

func TestCallsListPassesQueryAndReadsCursor(t *testing.T) {
	var query map[string]string
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/v1/calls" {
			http.NotFound(w, r)
			return
		}
		query = map[string]string{}
		for k := range r.URL.Query() {
			query[k] = r.URL.Query().Get(k)
		}
		_, _ = w.Write([]byte(`{"calls":[` + callJSON("c1", "active") + `,` + callJSON("c2", "active") + `],"next_cursor":"cur-2"}`))
	})
	page, err := c.Calls().List(context.Background(), ListCallsOptions{
		AgentID: "a1", State: CallActive, Limit: 2, Cursor: "cur-1",
	})
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	want := map[string]string{"agent_id": "a1", "state": "active", "limit": "2", "cursor": "cur-1"}
	if len(query) != len(want) {
		t.Errorf("query = %v, want %v", query, want)
	}
	for k, v := range want {
		if query[k] != v {
			t.Errorf("query[%q] = %q, want %q", k, query[k], v)
		}
	}
	if len(page.Calls) != 2 || page.Calls[1].ID != "c2" || page.Calls[0].State != CallActive {
		t.Errorf("calls = %+v", page.Calls)
	}
	if page.NextCursor == nil || *page.NextCursor != "cur-2" {
		t.Errorf("next_cursor = %v, want cur-2", page.NextCursor)
	}
}

func TestCallsSendAndHistoryRoundTrip(t *testing.T) {
	var sent map[string]any
	var historyQuery string
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/calls/c1/messages":
			sent = readBody(t, r)
			w.WriteHeader(http.StatusCreated)
			_, _ = w.Write([]byte(`{"seq":4,"kind":"message","from_agent_id":"a1","text":"Tuesday at 10?","data":{"slot":"tue-10"},"blocked":false,"created_at":"2026-10-26T10:01:00Z"}`))
		case r.Method == http.MethodGet && r.URL.Path == "/v1/calls/c1/messages":
			historyQuery = r.URL.RawQuery
			_, _ = w.Write([]byte(`[` +
				`{"seq":4,"kind":"message","from_agent_id":"a1","text":"Tuesday at 10?","data":{"slot":"tue-10"},"blocked":false,"created_at":"2026-10-26T10:01:00Z"},` +
				`{"seq":5,"kind":"message","from_agent_id":"a2","text":"Mail me at [contact removed]","data":null,"blocked":false,"safety":[{"kind":"email","action":"redacted","path":"message.text"}],"created_at":"2026-10-26T10:02:00Z"},` +
				`{"seq":6,"kind":"invocation","from_agent_id":"a1","text":null,"data":null,"blocked":false,"created_at":"2026-10-26T10:03:00Z","invocation":{"id":"inv1","capability":"book_room","status":"succeeded"}}` +
				`]`))
		default:
			http.NotFound(w, r)
		}
	})
	ctx := context.Background()
	entry, err := c.Calls().Send(ctx, "c1", "a1", CallMessage{Text: "Tuesday at 10?", Data: json.RawMessage(`{"slot":"tue-10"}`)})
	if err != nil {
		t.Fatalf("Send: %v", err)
	}
	if sent["agent_id"] != "a1" || sent["text"] != "Tuesday at 10?" {
		t.Errorf("sent = %v", sent)
	}
	if data, _ := sent["data"].(map[string]any); data["slot"] != "tue-10" {
		t.Errorf("sent data = %v", sent["data"])
	}
	if entry.Seq != 4 || entry.Kind != CallEntryMessage || entry.Text == nil || *entry.Text != "Tuesday at 10?" ||
		string(entry.Data) != `{"slot":"tue-10"}` {
		t.Errorf("entry = %+v", entry)
	}

	entries, err := c.Calls().History(ctx, "c1", CallHistoryOptions{After: 3, Limit: 10, Q: "tuesday"})
	if err != nil {
		t.Fatalf("History: %v", err)
	}
	if historyQuery != "after=3&limit=10&q=tuesday" {
		t.Errorf("history query = %q", historyQuery)
	}
	if len(entries) != 3 {
		t.Fatalf("got %d entries, want 3", len(entries))
	}
	if entries[0].Seq != entry.Seq || *entries[0].Text != *entry.Text {
		t.Errorf("sent entry didn't round trip: %+v", entries[0])
	}
	if s := entries[1].Safety; len(s) != 1 || s[0].Kind != "email" || s[0].Action != "redacted" {
		t.Errorf("safety = %+v", s)
	}
	inv := entries[2].Invocation
	if entries[2].Kind != CallEntryInvocation || inv == nil || inv.ID != "inv1" ||
		*inv.Capability != "book_room" || *inv.Status != InvocationSucceeded || entries[2].Text != nil {
		t.Errorf("invocation entry = %+v (%+v)", entries[2], inv)
	}
}

func TestCallsAskParsesRecall(t *testing.T) {
	var body map[string]any
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/v1/calls/c1/memory/ask" {
			http.NotFound(w, r)
			return
		}
		body = readBody(t, r)
		_, _ = w.Write([]byte(`{"answered":0.93,"messages":[` +
			`{"seq":7,"from":"a2","text":"Booked: Tuesday 10:00, room 4.","at":"2026-10-26T10:05:00Z","p":0.88},` +
			`{"seq":4,"from":"a1","text":"Tuesday at 10?","at":"2026-10-26T10:01:00Z","p":0.07}]}`))
	})
	rec, err := c.Calls().Ask(context.Background(), "c1", "a1", "When is the meeting?")
	if err != nil {
		t.Fatalf("Ask: %v", err)
	}
	if body["agent_id"] != "a1" || body["question"] != "When is the meeting?" {
		t.Errorf("body = %v", body)
	}
	if rec.Answered == nil || *rec.Answered != 0.93 {
		t.Errorf("answered = %v, want 0.93", rec.Answered)
	}
	if len(rec.Messages) != 2 {
		t.Fatalf("messages = %+v", rec.Messages)
	}
	m := rec.Messages[0]
	if m.Seq != 7 || *m.From != "a2" || *m.Text != "Booked: Tuesday 10:00, room 4." || m.P != 0.88 ||
		!m.At.Equal(time.Date(2026, 10, 26, 10, 5, 0, 0, time.UTC)) {
		t.Errorf("message = %+v", m)
	}
}

func TestCallsAskWithNothingToSearch(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{"answered":null,"messages":[]}`))
	})
	rec, err := c.Calls().Ask(context.Background(), "c1", "a1", "anything?")
	if err != nil {
		t.Fatalf("Ask: %v", err)
	}
	if rec.Answered != nil || len(rec.Messages) != 0 {
		t.Errorf("recall = %+v", rec)
	}
}

func TestCallsMemoryParsesBundle(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/v1/calls/c1/memory" {
			http.NotFound(w, r)
			return
		}
		_, _ = w.Write([]byte(`{` +
			`"pinned":[{"seq":3,"from":"a2","at":"2026-10-26T10:00:30Z","text":"Agreed.","context":"Budget is 4,000 EUR","tags":{"decision":0.91,"commitment":0.2,"open_question":0.01,"fact":0.3,"weight":2.7}}],` +
			`"open":[{"seq":2,"from":"a1","at":"2026-10-26T10:00:20Z","text":"Who books the train?"}],` +
			`"recent":[{"seq":6,"from":"a1","at":"2026-10-26T10:03:00Z","invocation":{"id":"inv1","capability":"book_room","status":"succeeded","input":{"room":4},"output":{"ok":true}}}]}`))
	})
	mem, err := c.Calls().Memory(context.Background(), "c1")
	if err != nil {
		t.Fatalf("Memory: %v", err)
	}
	if len(mem.Pinned) != 1 || len(mem.Open) != 1 || len(mem.Recent) != 1 {
		t.Fatalf("memory = %+v", mem)
	}
	p := mem.Pinned[0]
	if *p.Text != "Agreed." || *p.Context != "Budget is 4,000 EUR" || p.Tags == nil || p.Tags.Decision != 0.91 || p.Tags.Weight != 2.7 {
		t.Errorf("pinned = %+v (tags %+v)", p, p.Tags)
	}
	inv := mem.Recent[0].Invocation
	if inv == nil || inv.Capability != "book_room" || inv.Status != InvocationSucceeded || string(inv.Output) != `{"ok":true}` {
		t.Errorf("recent invocation = %+v", inv)
	}
}

func TestCallsErrorCodeSurfaces(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "application/json")
		w.WriteHeader(http.StatusConflict)
		_, _ = w.Write([]byte(`{"error":{"code":"chk.calls.not_active","message":"the call isn't active"}}`))
	})
	_, err := c.Calls().End(context.Background(), "c1", "a1")
	var apiErr *Error
	if !errors.As(err, &apiErr) {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if apiErr.Status != http.StatusConflict || apiErr.Code != "chk.calls.not_active" || apiErr.Message != "the call isn't active" {
		t.Errorf("got %+v", apiErr)
	}
}

// feedItem is one calls-feed item as the relay sends it.
func feedItem(callID, state string, seq int, kind, from, text, extra string) string {
	textJSON := "null"
	if text != "" {
		textJSON = `"` + text + `"`
	}
	return `{"call":` + callJSON(callID, state) + `,"entry":{"seq":` + strconv.Itoa(seq) + `,"kind":"` + kind +
		`","from_agent_id":"` + from + `","text":` + textJSON + `,"data":null,"blocked":false,"created_at":"2026-10-26T10:00:00Z"}` +
		extra + `}`
}

// callsRelay is a fake relay for Serve: the first pull returns batch,
// later ones nothing; it records answers and sends. idle closes on the
// third pull: Serve handled the whole batch, found the feed empty, slept
// and pulled again.
type callsRelay struct {
	t        *testing.T
	batch    string
	pulls    atomic.Int32
	idle     chan struct{}
	idleOnce sync.Once
	mu       sync.Mutex
	posts    map[string]map[string]any // path → body
	limits   []string
	refused  map[string]bool // paths answered 409 chk.calls.not_active
}

func newCallsRelay(t *testing.T, batch string, refused map[string]bool) *callsRelay {
	return &callsRelay{
		t:       t,
		batch:   batch,
		idle:    make(chan struct{}),
		posts:   map[string]map[string]any{},
		refused: refused,
	}
}

func (f *callsRelay) handle(w http.ResponseWriter, r *http.Request) {
	switch {
	case r.Method == http.MethodGet && r.URL.Path == "/v1/inbox/calls":
		f.mu.Lock()
		f.limits = append(f.limits, r.URL.Query().Get("limit"))
		f.mu.Unlock()
		if r.URL.Query().Get("agent_id") != "a2" {
			f.t.Errorf("pulled for agent %q", r.URL.Query().Get("agent_id"))
		}
		n := f.pulls.Add(1)
		if n >= 3 {
			f.idleOnce.Do(func() { close(f.idle) })
		}
		if n > 1 {
			_, _ = w.Write([]byte(`[]`))
			return
		}
		_, _ = w.Write([]byte(f.batch))
	case r.Method == http.MethodPost && strings.HasPrefix(r.URL.Path, "/v1/calls/"):
		body := readBody(f.t, r)
		f.mu.Lock()
		f.posts[r.URL.Path] = body
		f.mu.Unlock()
		if f.refused[r.URL.Path] {
			w.WriteHeader(http.StatusConflict)
			_, _ = w.Write([]byte(`{"error":{"code":"chk.calls.not_active","message":"the call isn't active"}}`))
			return
		}
		if strings.HasSuffix(r.URL.Path, "/messages") {
			w.WriteHeader(http.StatusCreated)
			_, _ = w.Write([]byte(`{"seq":9,"kind":"message","from_agent_id":"a2","text":"ok","data":null,"blocked":false,"created_at":"2026-10-26T10:00:00Z"}`))
			return
		}
		id := strings.Split(strings.TrimPrefix(r.URL.Path, "/v1/calls/"), "/")[0]
		_, _ = w.Write([]byte(callJSON(id, "active")))
	default:
		http.NotFound(w, r)
	}
}

func (f *callsRelay) post(path string) (map[string]any, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	b, ok := f.posts[path]
	return b, ok
}

// runServe runs Serve until done is closed (or 2 s pass), then cancels
// it and checks that it stops.
func runServe(t *testing.T, c *Client, opts CallsServeOptions, done <-chan struct{}) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	stopped := make(chan error, 1)
	go func() { stopped <- c.Calls().Serve(ctx, "a2", opts) }()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Error("Serve didn't handle the batch in time")
	}
	cancel()
	select {
	case err := <-stopped:
		if err != nil {
			t.Fatalf("Serve: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("Serve did not stop on cancel")
	}
}

func TestCallsServeHandlesTheFeed(t *testing.T) {
	memory := `,"memory":{"pinned":[],"open":[],"recent":[{"seq":2,"from":"a1","at":"2026-10-26T10:00:00Z","text":"hello"}]}`
	relay := newCallsRelay(t, `[`+
		feedItem("c0", "active", 1, "message", "a1", "boom", "")+`,`+
		feedItem("c1", "ringing", 1, "invite", "a1", "Got a minute?", "")+`,`+
		feedItem("c2", "active", 2, "message", "a1", "hello", memory)+`,`+
		feedItem("c3", "ended", 5, "ended", "a1", "", "")+`,`+
		feedItem("c4", "ended", 3, "message", "a1", "too late", "")+
		`]`, map[string]bool{"/v1/calls/c4/messages": true})
	_, c := newServer(t, relay.handle)

	var mu sync.Mutex
	var events []CallFeedItem
	var errs []error
	var errItems []string
	var gotMemory *CallMemory
	opts := CallsServeOptions{
		PollInterval: 10 * time.Millisecond,
		OnMessage: func(_ context.Context, item CallFeedItem) (*CallMessage, error) {
			switch item.Call.ID {
			case "c0":
				return nil, errors.New("handler failed")
			case "c2":
				mu.Lock()
				gotMemory = item.Memory
				mu.Unlock()
				return &CallMessage{Text: "re: " + *item.Entry.Text}, nil
			case "c4":
				return &CallMessage{Text: "sorry, late"}, nil
			}
			t.Errorf("unexpected message on %s", item.Call.ID)
			return nil, nil
		},
		OnEvent: func(_ context.Context, item CallFeedItem) error {
			mu.Lock()
			events = append(events, item)
			mu.Unlock()
			return nil
		},
		OnError: func(err error, item *CallFeedItem) {
			mu.Lock()
			defer mu.Unlock()
			errs = append(errs, err)
			if item != nil {
				errItems = append(errItems, item.Call.ID)
			}
		},
	}
	runServe(t, c, opts, relay.idle)

	// Accepted by default, with no first reply.
	if body, ok := relay.post("/v1/calls/c1/accept"); !ok || body["agent_id"] != "a2" || body["message"] != nil {
		t.Errorf("accept = %v (posted: %v)", body, ok)
	}
	// Replied through OnMessage, with the memory bundle.
	if body, ok := relay.post("/v1/calls/c2/messages"); !ok || body["agent_id"] != "a2" || body["text"] != "re: hello" {
		t.Errorf("reply = %v (posted: %v)", body, ok)
	}
	mu.Lock()
	defer mu.Unlock()
	if gotMemory == nil || len(gotMemory.Recent) != 1 || *gotMemory.Recent[0].Text != "hello" {
		t.Errorf("memory = %+v", gotMemory)
	}
	// OnEvent for the end.
	if len(events) != 1 || events[0].Call.ID != "c3" || events[0].Entry.Kind != CallEntryEnded {
		t.Errorf("events = %+v", events)
	}
	// The handler error was reported and the loop went on; the reply to
	// the call that ended meanwhile was dropped quietly.
	if _, ok := relay.post("/v1/calls/c4/messages"); !ok {
		t.Error("no reply attempted on c4")
	}
	if len(errs) != 1 || errs[0].Error() != "handler failed" || len(errItems) != 1 || errItems[0] != "c0" {
		t.Errorf("errors = %v on %v", errs, errItems)
	}
	if _, ok := relay.post("/v1/calls/c0/messages"); ok {
		t.Error("replied on c0 though its handler failed")
	}
	if relay.limits[0] != "25" {
		t.Errorf("batch limit = %q, want 25", relay.limits[0])
	}
}

func TestCallsServeDeclinesThroughOnInviteAndSurvivesAPanic(t *testing.T) {
	relay := newCallsRelay(t, `[`+
		feedItem("c0", "active", 1, "message", "a1", "panic please", "")+`,`+
		feedItem("c1", "ringing", 1, "invite", "a1", "Buy my course?", "")+
		`]`, nil)
	_, c := newServer(t, relay.handle)

	var panics atomic.Int32
	runServe(t, c, CallsServeOptions{
		PollInterval: 10 * time.Millisecond,
		BatchSize:    5,
		OnMessage: func(context.Context, CallFeedItem) (*CallMessage, error) {
			panic("handler bug")
		},
		OnInvite: func(_ context.Context, item CallFeedItem) (CallAnswer, error) {
			if *item.Entry.Text != "Buy my course?" {
				t.Errorf("invite text = %q", *item.Entry.Text)
			}
			return DeclineCall("not interested"), nil
		},
		OnError: func(err error, item *CallFeedItem) {
			if item != nil && item.Call.ID == "c0" && strings.Contains(err.Error(), "handler bug") {
				panics.Add(1)
				return
			}
			t.Errorf("unexpected error: %v", err)
		},
	}, relay.idle)

	if body, ok := relay.post("/v1/calls/c1/decline"); !ok || body["agent_id"] != "a2" || body["reason"] != "not interested" {
		t.Errorf("decline = %v (posted: %v)", body, ok)
	}
	if _, ok := relay.post("/v1/calls/c1/accept"); ok {
		t.Error("accepted a declined invite")
	}
	if panics.Load() != 1 {
		t.Errorf("panic reports = %d, want 1", panics.Load())
	}
	if relay.limits[0] != "5" {
		t.Errorf("batch limit = %q, want 5", relay.limits[0])
	}
}

func TestCallsServeRequiresOnMessage(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) { http.NotFound(w, r) })
	if err := c.Calls().Serve(context.Background(), "a2", CallsServeOptions{}); err == nil {
		t.Fatal("expected an error without OnMessage")
	}
}

func TestInvokeSendsCallID(t *testing.T) {
	var body map[string]any
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/invoke":
			body = readBody(t, r)
			_, _ = w.Write([]byte(`{"invocation_id":"inv1","status":"pending","error":null}`))
		case r.Method == http.MethodGet && r.URL.Path == "/v1/invocations/inv1":
			_, _ = w.Write([]byte(`{"id":"inv1","capability_name":"book_room","status":"pending","elapsed_ms":0,"input_preview":{},"created_at":"2026-10-26T10:00:00Z","i_served":false,"i_invoked":true,"call_id":"c1"}`))
		default:
			http.NotFound(w, r)
		}
	})
	ctx := context.Background()
	if _, err := c.Invoke(ctx, &InvokeRequest{
		GrantID:        "g1",
		GranteeAgentID: "a1",
		Input:          json.RawMessage(`{"room":4}`),
		CallID:         "c1",
	}); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if body["call_id"] != "c1" || body["grant_id"] != "g1" {
		t.Errorf("body = %v", body)
	}
	inv, err := c.Invocations().Get(ctx, "inv1")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if inv.CallID == nil || *inv.CallID != "c1" {
		t.Errorf("call_id = %v, want c1", inv.CallID)
	}

	// Without a call, call_id isn't sent.
	if _, err := c.Invoke(ctx, &InvokeRequest{GrantID: "g1", GranteeAgentID: "a1", Input: json.RawMessage(`{}`)}); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if _, ok := body["call_id"]; ok {
		t.Errorf("call_id sent without a call: %v", body)
	}
}

func TestAgentUpdateSendsAcceptCalls(t *testing.T) {
	var body map[string]any
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPatch || r.URL.Path != "/v1/agents/a1" {
			http.NotFound(w, r)
			return
		}
		body = readBody(t, r)
		_, _ = w.Write([]byte(`{"id":"a1","account_id":"acc1","account_slug":"alice","account_display_name":"Alice","slug":"scheduler","display_name":"Scheduler","description":"","visibility":"network","endpoint_url":null,"created_at":"2026-10-26T10:00:00Z","updated_at":"2026-10-26T10:00:00Z","is_mine":true,"capability_count":1,"avg_rating":null,"review_count":0,"accept_calls":"anyone"}`))
	})
	agent, err := c.Agents().Update(context.Background(), "a1", &UpdateAgentRequest{AcceptCalls: AcceptCallsAnyone})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if len(body) != 1 || body["accept_calls"] != "anyone" {
		t.Errorf("body = %v, want only accept_calls", body)
	}
	if agent.AcceptCalls != AcceptCallsAnyone {
		t.Errorf("accept_calls = %q", agent.AcceptCalls)
	}
}
