package chakramcp

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func newServer(t *testing.T, handler http.HandlerFunc) (*httptest.Server, *Client) {
	t.Helper()
	srv := httptest.NewServer(handler)
	t.Cleanup(srv.Close)
	c, err := NewWithOptions(Options{
		APIKey:   "ck_test",
		AppURL:   srv.URL,
		RelayURL: srv.URL,
	})
	if err != nil {
		t.Fatalf("NewWithOptions: %v", err)
	}
	return srv, c
}

func TestRejectsBadAPIKey(t *testing.T) {
	if _, err := New("not-a-key"); err == nil {
		t.Fatal("expected error for non-`ck_` key")
	}
}

func TestMeSetsBearer(t *testing.T) {
	var seenAuth string
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		seenAuth = r.Header.Get("authorization")
		_ = json.NewEncoder(w).Encode(map[string]any{
			"user": map[string]any{
				"id":           "u1",
				"email":        "alice@example.com",
				"display_name": "Alice",
				"avatar_url":   nil,
				"is_admin":     false,
			},
			"memberships":     []any{},
			"survey_required": false,
		})
	})
	me, err := c.Me(context.Background())
	if err != nil {
		t.Fatalf("Me: %v", err)
	}
	if me.User.Email != "alice@example.com" {
		t.Errorf("email = %q, want alice@example.com", me.User.Email)
	}
	if seenAuth != "Bearer ck_test" {
		t.Errorf("authorization = %q, want %q", seenAuth, "Bearer ck_test")
	}
}

func TestNetworkSendsLimitAndCursorAndDecodesPage(t *testing.T) {
	const agentJSON = `{"id":"a1","account_id":"acc1","account_slug":"alice","account_display_name":"Alice",` +
		`"slug":"scheduler","display_name":"Scheduler","description":"Books meetings","mode":"pull",` +
		`"accept_calls":"friends","tags":["calendar"],"verified":true,"endpoint_url":null,` +
		`"created_at":"2026-10-26T10:00:00Z","updated_at":"2026-10-26T10:00:00Z","is_mine":false,` +
		`"capability_count":2,"avg_rating":4.5,"review_count":3,"online":true,` +
		`"visibility":"network","capabilities":[{"id":"cap1","name":"book","access":"friends"}],` +
		`"last_seen_at":"2026-10-27T09:00:00Z","friend_count":4,"featured":true}`
	var queries []map[string]string
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/v1/network/agents" {
			http.NotFound(w, r)
			return
		}
		q := map[string]string{}
		for k := range r.URL.Query() {
			q[k] = r.URL.Query().Get(k)
		}
		queries = append(queries, q)
		if len(q) > 0 {
			_, _ = w.Write([]byte(`{"agents":[` + agentJSON + `],"next_cursor":"cur-2"}`))
			return
		}
		_, _ = w.Write([]byte(`{"agents":[]}`))
	})

	page, err := c.Network(context.Background(), NetworkOptions{Limit: 1, Cursor: "cur 1"})
	if err != nil {
		t.Fatalf("Network: %v", err)
	}
	if queries[0]["limit"] != "1" || queries[0]["cursor"] != "cur 1" || len(queries[0]) != 2 {
		t.Errorf("query = %v, want limit=1 cursor=%q", queries[0], "cur 1")
	}
	if page.NextCursor == nil || *page.NextCursor != "cur-2" {
		t.Errorf("next_cursor = %v, want cur-2", page.NextCursor)
	}
	if len(page.Agents) != 1 {
		t.Fatalf("agents = %+v, want 1", page.Agents)
	}
	a := page.Agents[0]
	if a.Slug != "scheduler" || a.Mode != AgentModePull || !a.Verified || !a.Online ||
		a.CapabilityCount != 2 || a.AcceptCalls != AcceptCallsFriends ||
		len(a.Tags) != 1 || a.Tags[0] != "calendar" || a.AvgRating == nil || *a.AvgRating != 4.5 {
		t.Errorf("agent = %+v", a)
	}
	if a.Visibility != VisibilityNetwork || len(a.Capabilities) != 1 || a.Capabilities[0].Access != AccessFriends ||
		a.LastSeenAt == nil || a.FriendCount != 4 || !a.Featured {
		t.Errorf("directory fields = %+v", a)
	}

	// No options: no query; the last page has no next_cursor.
	last, err := c.Network(context.Background(), NetworkOptions{})
	if err != nil {
		t.Fatalf("Network (last page): %v", err)
	}
	if len(queries[1]) != 0 {
		t.Errorf("query = %v, want none", queries[1])
	}
	if len(last.Agents) != 0 || last.NextCursor != nil {
		t.Errorf("last page = %+v, want empty with no cursor", last)
	}
}

func TestErrorEnvelopeDecoded(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "application/json")
		w.WriteHeader(http.StatusForbidden)
		_, _ = w.Write([]byte(`{"error":{"code":"forbidden","message":"forbidden"}}`))
	})
	_, err := c.Agents().List(context.Background())
	apiErr, ok := err.(*Error)
	if !ok {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if apiErr.Status != 403 || apiErr.Code != "forbidden" {
		t.Errorf("got %+v", apiErr)
	}
}

func TestRefusedInvokeCarriesReasonAndCode(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "application/json")
		w.WriteHeader(http.StatusForbidden)
		_, _ = w.Write([]byte(`{"invocation_id":"inv","status":"rejected","error":"System One safety check blocked this request: threatens violence","code":"chk.policy.safety_blocked"}`))
	})
	_, err := c.Invoke(context.Background(), &InvokeRequest{GranteeAgentID: "a2", Input: json.RawMessage(`{}`)})
	apiErr, ok := err.(*Error)
	if !ok {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if apiErr.Status != 403 || apiErr.Code != "chk.policy.safety_blocked" ||
		apiErr.Message != "System One safety check blocked this request: threatens violence" {
		t.Errorf("got %+v", apiErr)
	}
}

func TestInvokeAndWaitPollsUntilTerminal(t *testing.T) {
	var pollCount atomic.Int32
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/invoke":
			_, _ = w.Write([]byte(`{"invocation_id":"inv1","status":"pending","error":null}`))
		case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/invocations/inv1"):
			n := pollCount.Add(1)
			status := "in_progress"
			out := "null"
			if n >= 2 {
				status = "succeeded"
				out = `{"echoed":"world"}`
			}
			_, _ = w.Write([]byte(`{"id":"inv1","grant_id":"g1","granter_agent_id":"a1","granter_display_name":"Alice","grantee_agent_id":"a2","grantee_display_name":"Bob","capability_id":"c1","capability_name":"echo","status":"` + status + `","elapsed_ms":100,"error_message":null,"input_preview":{"hello":"world"},"output_preview":` + out + `,"created_at":"2026-01-01T00:00:00Z","claimed_at":null,"i_served":false,"i_invoked":true}`))
		default:
			http.NotFound(w, r)
		}
	})
	final, err := c.InvokeAndWait(context.Background(),
		&InvokeRequest{
			GrantID:        "g1",
			GranteeAgentID: "a2",
			Input:          json.RawMessage(`{"hello":"world"}`),
		},
		PollOptions{Interval: 5 * time.Millisecond, Timeout: 5 * time.Second})
	if err != nil {
		t.Fatalf("InvokeAndWait: %v", err)
	}
	if final.Status != InvocationSucceeded {
		t.Errorf("status = %q, want succeeded", final.Status)
	}
	if string(final.OutputPreview) != `{"echoed":"world"}` {
		t.Errorf("output = %q", string(final.OutputPreview))
	}
}

func TestInboxServeDispatchesAndCancels(t *testing.T) {
	var responded atomic.Int32
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/inbox"):
			_, _ = w.Write([]byte(`[{"id":"inv1","grant_id":null,"granter_agent_id":null,"granter_display_name":null,"grantee_agent_id":null,"grantee_display_name":null,"capability_id":null,"capability_name":"echo","status":"in_progress","elapsed_ms":0,"error_message":null,"input_preview":{"hi":"there"},"output_preview":null,"created_at":"2026-01-01T00:00:00Z","claimed_at":"2026-01-01T00:00:01Z","i_served":true,"i_invoked":false}]`))
		case r.Method == http.MethodPost && r.URL.Path == "/v1/invocations/inv1/result":
			responded.Add(1)
			_, _ = w.Write([]byte(`{}`))
		default:
			http.NotFound(w, r)
		}
	})
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)

	handler := func(_ context.Context, inv Invocation) (HandlerResult, error) {
		if inv.ID != "inv1" {
			t.Errorf("got id %q", inv.ID)
		}
		return Succeeded(map[string]any{"ok": true}), nil
	}

	done := make(chan error, 1)
	go func() {
		done <- c.Inbox().Serve(ctx, "agent-id", handler, ServeOptions{PollInterval: 20 * time.Millisecond})
	}()

	// Wait until the server has recorded at least one response, then
	// cancel the loop. Bounded poll so the test fails fast if Serve
	// never actually dispatches.
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if responded.Load() > 0 {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	cancel()

	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("Serve: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("Serve did not exit on cancel")
	}

	if responded.Load() == 0 {
		t.Errorf("expected at least one response, got %d", responded.Load())
	}
}

func TestServeDraftsHumanRepliesAndPassesNotices(t *testing.T) {
	var pulls atomic.Int32
	var posted atomic.Value
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.URL.Path == "/v1/inbox/notices":
			_, _ = w.Write([]byte(`[{"id":"n1","kind":"consent_requested","created_at":"2026-10-23T00:00:00Z","consent_request":{"id":"cr-9","invocation_id":"inv9","status":"pending","capability_id":"cap","capability_name":"book_meeting","requester":{"agent_id":"a","agent_display_name":"Scheduler","account_id":"acc","account_display_name":null,"verified":true},"terms":{"valid_for":"once"},"expires_at":"2026-10-24T00:00:00Z","approve_url":"https://chakramcp.com/app/approvals/cr-9"}}]`))
		case r.URL.Path == "/v1/inbox":
			if pulls.Add(1) > 1 {
				_, _ = w.Write([]byte(`[]`))
				return
			}
			_, _ = w.Write([]byte(`[{"id":"inv1","capability_name":"message_owner","status":"in_progress","elapsed_ms":0,"input_preview":{},"created_at":"2026-01-01T00:00:00Z","i_served":true,"i_invoked":false,"semantics":"human_in_loop"}]`))
		case r.Method == http.MethodPost && r.URL.Path == "/v1/invocations/inv1/result":
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			posted.Store(body["output"])
			w.WriteHeader(http.StatusAccepted)
			_, _ = w.Write([]byte(`{"status":"awaiting_confirmation","consent_request_id":"cr-1","approve_url":"https://chakramcp.com/app/approvals/cr-1","expires_at":"2026-10-24T00:00:00Z"}`))
		default:
			http.NotFound(w, r)
		}
	})
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)

	drafts := make(chan DraftedReply, 1)
	notices := make(chan Notice, 1)
	opts := ServeOptions{
		PollInterval: 20 * time.Millisecond,
		HumanHandler: func(_ context.Context, inv Invocation) (*HandlerResult, error) {
			r := Succeeded(map[string]any{"reply": "Tuesday works"})
			return &r, nil
		},
		OnDraft:  func(_ Invocation, d DraftedReply) { drafts <- d },
		OnNotice: func(n Notice) { notices <- n },
	}
	autonomous := func(context.Context, Invocation) (HandlerResult, error) {
		t.Error("a human_in_loop call reached the autonomous handler")
		return Failed("no"), nil
	}
	done := make(chan error, 1)
	go func() { done <- c.Inbox().Serve(ctx, "agent-id", autonomous, opts) }()

	select {
	case d := <-drafts:
		if d.ConsentRequestID != "cr-1" || d.ApproveURL != "https://chakramcp.com/app/approvals/cr-1" {
			t.Errorf("draft = %+v", d)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no draft")
	}
	select {
	case n := <-notices:
		if n.Kind != NoticeConsentRequested || n.ConsentRequest == nil || *n.ConsentRequest.CapabilityName != "book_meeting" {
			t.Errorf("notice = %+v", n)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no notice")
	}
	cancel()
	if err := <-done; err != nil {
		t.Fatalf("Serve: %v", err)
	}
	if got, _ := posted.Load().(map[string]any); got["reply"] != "Tuesday works" {
		t.Errorf("posted output = %v", got)
	}
}

func TestRespondReturnsTheFinishedInvocation(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`{"id":"inv1","capability_name":"echo","status":"succeeded","elapsed_ms":3,"input_preview":{},"created_at":"2026-01-01T00:00:00Z","i_served":true,"i_invoked":false}`))
	})
	res, err := c.Inbox().Respond(context.Background(), "inv1", Succeeded("hi"))
	if err != nil {
		t.Fatal(err)
	}
	if res.Drafted != nil || res.Invocation == nil || res.Invocation.Status != InvocationSucceeded {
		t.Errorf("result = %+v", res)
	}
}

// ─── Usage limits ─────────────────────────────────────────

// invokeError runs Invoke against a server that answers with one
// canned error and returns what it got back.
func invokeError(t *testing.T, status int, headers map[string]string, body string) error {
	t.Helper()
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "application/json")
		for k, v := range headers {
			w.Header().Set(k, v)
		}
		w.WriteHeader(status)
		_, _ = w.Write([]byte(body))
	})
	_, err := c.Invoke(context.Background(), &InvokeRequest{
		CapabilityID:   "cap",
		GranteeAgentID: "agent",
		Input:          json.RawMessage(`{}`),
	})
	if err == nil {
		t.Fatal("expected an error")
	}
	return err
}

// assertRetryAfter checks the retry delay on the wrapped *Error, which
// errors.As must still reach.
func assertRetryAfter(t *testing.T, err error, want *int64) {
	t.Helper()
	var apiErr *Error
	if !errors.As(err, &apiErr) {
		t.Fatalf("errors.As(*Error) failed for %T: %v", err, err)
	}
	switch {
	case want == nil && apiErr.RetryAfterSecs != nil:
		t.Errorf("RetryAfterSecs = %d, want nil", *apiErr.RetryAfterSecs)
	case want != nil && apiErr.RetryAfterSecs == nil:
		t.Errorf("RetryAfterSecs = nil, want %d", *want)
	case want != nil && *apiErr.RetryAfterSecs != *want:
		t.Errorf("RetryAfterSecs = %d, want %d", *apiErr.RetryAfterSecs, *want)
	}
}

func secs(n int64) *int64 { return &n }

func TestRateLimitedUsesRetryAfterHeader(t *testing.T) {
	err := invokeError(t, http.StatusTooManyRequests, map[string]string{"Retry-After": "23"},
		`{"error":{"code":"account_rate_limited","message":"rate limit exceeded","retryable":true,"retry_after_secs":30}}`)
	var rl *RateLimitedError
	if !errors.As(err, &rl) {
		t.Fatalf("expected *RateLimitedError, got %T: %v", err, err)
	}
	if rl.Err.Status != 429 || rl.Err.Code != "account_rate_limited" || rl.Err.Message != "rate limit exceeded" {
		t.Errorf("got %+v", rl.Err)
	}
	// The header wins over the body.
	assertRetryAfter(t, err, secs(23))
}

func TestCreditsExhaustedWithCreditsURL(t *testing.T) {
	err := invokeError(t, http.StatusTooManyRequests, map[string]string{"Retry-After": "1300000"},
		`{"error":{"code":"account_credits_exhausted","message":"insufficient credits","retryable":false,`+
			`"retry_after_secs":1300000,"next_grant_at":"2026-11-01T00:00:00Z","credits_url":"https://chakramcp.com/app/credits"}}`)
	var ce *CreditsExhaustedError
	if !errors.As(err, &ce) {
		t.Fatalf("expected *CreditsExhaustedError, got %T: %v", err, err)
	}
	if want := time.Date(2026, 11, 1, 0, 0, 0, 0, time.UTC); !ce.NextGrantAt.Equal(want) {
		t.Errorf("NextGrantAt = %s, want %s", ce.NextGrantAt, want)
	}
	if ce.CreditsURL != "https://chakramcp.com/app/credits" {
		t.Errorf("CreditsURL = %q", ce.CreditsURL)
	}
	assertRetryAfter(t, err, secs(1300000))
}

func TestCreditsExhaustedSelfHostedHasNoCreditsURL(t *testing.T) {
	err := invokeError(t, http.StatusTooManyRequests, map[string]string{"Retry-After": "86400"},
		`{"error":{"code":"account_credits_exhausted","message":"insufficient credits","retryable":false,`+
			`"retry_after_secs":86400,"next_grant_at":"2026-11-01T00:00:00Z"}}`)
	var ce *CreditsExhaustedError
	if !errors.As(err, &ce) {
		t.Fatalf("expected *CreditsExhaustedError, got %T: %v", err, err)
	}
	if ce.CreditsURL != "" {
		t.Errorf("CreditsURL = %q, want empty", ce.CreditsURL)
	}
	if ce.NextGrantAt.IsZero() {
		t.Error("NextGrantAt is zero, want 2026-11-01")
	}
	assertRetryAfter(t, err, secs(86400))
}

func TestRetryAfterFromBodyWithoutHeader(t *testing.T) {
	err := invokeError(t, http.StatusTooManyRequests, nil,
		`{"error":{"code":"account_rate_limited","message":"rate limit exceeded","retryable":true,"retry_after_secs":17}}`)
	var rl *RateLimitedError
	if !errors.As(err, &rl) {
		t.Fatalf("expected *RateLimitedError, got %T: %v", err, err)
	}
	assertRetryAfter(t, err, secs(17))
}

func TestQuotaExhaustedGainsRetryAfter(t *testing.T) {
	err := invokeError(t, http.StatusTooManyRequests, map[string]string{"Retry-After": "1300000"},
		`{"error":{"code":"monthly_quota_exhausted","message":"monthly quota exhausted","retryable":false},`+
			`"quota":100,"resets_at":"2026-11-01T00:00:00Z"}`)
	var qe *QuotaExhaustedError
	if !errors.As(err, &qe) {
		t.Fatalf("expected *QuotaExhaustedError, got %T: %v", err, err)
	}
	if qe.Quota != 100 {
		t.Errorf("Quota = %d, want 100", qe.Quota)
	}
	assertRetryAfter(t, err, secs(1300000))
}

func TestOlderServerLimitErrorsHaveNoNewFields(t *testing.T) {
	err := invokeError(t, http.StatusTooManyRequests, nil,
		`{"error":{"code":"account_credits_exhausted","message":"insufficient credits","retryable":false}}`)
	var ce *CreditsExhaustedError
	if !errors.As(err, &ce) {
		t.Fatalf("expected *CreditsExhaustedError, got %T: %v", err, err)
	}
	if !ce.NextGrantAt.IsZero() || ce.CreditsURL != "" {
		t.Errorf("got NextGrantAt=%s CreditsURL=%q, want zero values", ce.NextGrantAt, ce.CreditsURL)
	}
	assertRetryAfter(t, err, nil)

	err = invokeError(t, http.StatusTooManyRequests, nil,
		`{"error":{"code":"account_rate_limited","message":"rate limit exceeded"}}`)
	var rl *RateLimitedError
	if !errors.As(err, &rl) {
		t.Fatalf("expected *RateLimitedError, got %T: %v", err, err)
	}
	assertRetryAfter(t, err, nil)

	err = invokeError(t, http.StatusTooManyRequests, nil,
		`{"error":{"code":"monthly_quota_exhausted","message":"monthly quota exhausted"},"quota":100,"resets_at":"2026-11-01T00:00:00Z"}`)
	var qe *QuotaExhaustedError
	if !errors.As(err, &qe) {
		t.Fatalf("expected *QuotaExhaustedError, got %T: %v", err, err)
	}
	assertRetryAfter(t, err, nil)
}

func TestOtherErrorsCarryRetryAfterOnlyWhenSent(t *testing.T) {
	err := invokeError(t, http.StatusForbidden, nil, `{"error":{"code":"forbidden","message":"forbidden"}}`)
	if _, ok := err.(*Error); !ok {
		t.Fatalf("expected plain *Error, got %T: %v", err, err)
	}
	assertRetryAfter(t, err, nil)

	err = invokeError(t, http.StatusServiceUnavailable, map[string]string{"Retry-After": "5"},
		`{"error":{"code":"unavailable","message":"try later"}}`)
	if _, ok := err.(*Error); !ok {
		t.Fatalf("expected plain *Error, got %T: %v", err, err)
	}
	assertRetryAfter(t, err, secs(5))
}

// ─── Waiting pulls ────────────────────────────────────────

// pullLog records the URL of each pull a fake relay answers.
type pullLog struct {
	mu   sync.Mutex
	urls []*url.URL
}

func (p *pullLog) add(u *url.URL) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.urls = append(p.urls, u)
}

func (p *pullLog) count() int {
	p.mu.Lock()
	defer p.mu.Unlock()
	return len(p.urls)
}

func (p *pullLog) at(i int) *url.URL {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.urls[i]
}

// emptyFeed is a relay whose feed at path is always empty. It answers
// with the ChakraMCP-Wait header set to waited ("" sends none), and
// logs each pull.
func emptyFeed(path, waited string, pulls *pullLog) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != path {
			http.NotFound(w, r)
			return
		}
		pulls.add(r.URL)
		if waited != "" {
			w.Header().Set("ChakraMCP-Wait", waited)
		}
		_, _ = w.Write([]byte(`[]`))
	}
}

// startServe runs serve in the background. The returned stop cancels
// it and fails the test unless it returns nil within 2 s.
func startServe(t *testing.T, serve func(ctx context.Context) error) (stop func()) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	done := make(chan error, 1)
	go func() { done <- serve(ctx) }()
	return func() {
		t.Helper()
		cancel()
		select {
		case err := <-done:
			if err != nil {
				t.Fatalf("Serve: %v", err)
			}
		case <-time.After(2 * time.Second):
			t.Fatal("Serve did not stop on cancel")
		}
	}
}

// waitFor polls cond until it holds, failing the test after 2 s.
func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for !cond() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func intp(n int) *int { return &n }

func noopHandler(context.Context, Invocation) (HandlerResult, error) { return Succeeded(nil), nil }

func TestPullSendsWaitAndLeaseOnlyWhenSet(t *testing.T) {
	var pulls pullLog
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		pulls.add(r.URL)
		_, _ = w.Write([]byte(`[]`))
	})
	ctx := context.Background()
	if _, err := c.Inbox().PullWithOptions(ctx, "a1", PullOptions{Limit: 5, Wait: 20, LeaseSecs: 1800}); err != nil {
		t.Fatalf("PullWithOptions: %v", err)
	}
	if _, err := c.Inbox().Pull(ctx, "a1", 0); err != nil {
		t.Fatalf("Pull: %v", err)
	}
	if _, err := c.Inbox().PullWithMeta(ctx, "a1", PullOptions{Wait: 25}); err != nil {
		t.Fatalf("PullWithMeta: %v", err)
	}
	if _, err := c.Inbox().NoticesWithOptions(ctx, "a1", NoticesOptions{Wait: 25}); err != nil {
		t.Fatalf("NoticesWithOptions: %v", err)
	}
	if _, err := c.Inbox().Notices(ctx, "a1", 3); err != nil {
		t.Fatalf("Notices: %v", err)
	}
	want := []string{
		"/v1/inbox?agent_id=a1&lease_secs=1800&limit=5&wait=20",
		"/v1/inbox?agent_id=a1",
		"/v1/inbox?agent_id=a1&wait=25",
		"/v1/inbox/notices?agent_id=a1&wait=25",
		"/v1/inbox/notices?agent_id=a1&limit=3",
	}
	if pulls.count() != len(want) {
		t.Fatalf("pulls = %d, want %d", pulls.count(), len(want))
	}
	for i, w := range want {
		if got := pulls.at(i).RequestURI(); got != w {
			t.Errorf("pull %d = %s, want %s", i, got, w)
		}
	}
}

func TestPullWithMetaReadsTheWaitHeader(t *testing.T) {
	for _, tc := range []struct {
		name, header string
		want         *int
	}{
		{"waited", "25", intp(25)},
		{"did not wait", "0", intp(0)},
		{"older relay", "", nil},
		{"not an integer", "soon", nil},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
				if tc.header != "" {
					w.Header().Set("ChakraMCP-Wait", tc.header)
				}
				_, _ = w.Write([]byte(`[{"id":"inv1","capability_name":"echo","status":"in_progress","elapsed_ms":0,"input_preview":{},"created_at":"2026-01-01T00:00:00Z","i_served":true,"i_invoked":false}]`))
			})
			res, err := c.Inbox().PullWithMeta(context.Background(), "a1", PullOptions{Wait: 25})
			if err != nil {
				t.Fatalf("PullWithMeta: %v", err)
			}
			if len(res.Items) != 1 || res.Items[0].ID != "inv1" {
				t.Errorf("items = %+v", res.Items)
			}
			switch {
			case tc.want == nil && res.Waited != nil:
				t.Errorf("Waited = %d, want nil", *res.Waited)
			case tc.want != nil && (res.Waited == nil || *res.Waited != *tc.want):
				t.Errorf("Waited = %v, want %d", res.Waited, *tc.want)
			}
		})
	}
}

func TestPullWithMetaMapsErrorsAsBefore(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("ChakraMCP-Wait", "0")
		w.Header().Set("Retry-After", "12")
		w.WriteHeader(http.StatusTooManyRequests)
		_, _ = w.Write([]byte(`{"error":{"code":"account_rate_limited","message":"rate limit exceeded"}}`))
	})
	res, err := c.Inbox().PullWithMeta(context.Background(), "a1", PullOptions{Wait: 25})
	if res != nil {
		t.Errorf("result = %+v, want nil", res)
	}
	var rl *RateLimitedError
	if !errors.As(err, &rl) {
		t.Fatalf("expected *RateLimitedError, got %T: %v", err, err)
	}
	assertRetryAfter(t, err, secs(12))
}

func TestServeLoopsStraightBackWhileTheRelayWaits(t *testing.T) {
	var pulls pullLog
	_, c := newServer(t, emptyFeed("/v1/inbox", "25", &pulls))
	start := time.Now()
	stop := startServe(t, func(ctx context.Context) error {
		return c.Inbox().Serve(ctx, "a1", noopHandler, ServeOptions{PollInterval: 30 * time.Second, NoticeInterval: -1})
	})
	waitFor(t, "5 pulls without sleeping", func() bool { return pulls.count() >= 5 })
	stop()
	if elapsed := time.Since(start); elapsed > 5*time.Second {
		t.Errorf("took %s", elapsed)
	}
}

func TestServeSleepsWhenTheRelayDidNotWait(t *testing.T) {
	for _, tc := range []struct{ name, header string }{
		{"older relay", ""},
		{"did not wait", "0"},
		{"not an integer", "soon"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var pulls pullLog
			_, c := newServer(t, emptyFeed("/v1/inbox", tc.header, &pulls))
			stop := startServe(t, func(ctx context.Context) error {
				return c.Inbox().Serve(ctx, "a1", noopHandler, ServeOptions{PollInterval: 30 * time.Second, NoticeInterval: -1})
			})
			waitFor(t, "the first pull", func() bool { return pulls.count() >= 1 })
			time.Sleep(150 * time.Millisecond)
			stop() // also checks that cancelling cuts the sleep short
			if n := pulls.count(); n != 1 {
				t.Errorf("pulls = %d, want 1 (Serve should sleep PollInterval)", n)
			}
		})
	}
}

func TestServeSendsWaitAndLease(t *testing.T) {
	for _, tc := range []struct {
		name        string
		opts        ServeOptions
		wait, lease string // "" means not sent
	}{
		{"default", ServeOptions{}, "25", ""},
		{"own wait and lease", ServeOptions{WaitSecs: 5, LeaseSecs: 1800}, "5", "1800"},
		{"never wait", ServeOptions{WaitSecs: -1}, "", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var pulls pullLog
			_, c := newServer(t, emptyFeed("/v1/inbox", "", &pulls))
			opts := tc.opts
			opts.PollInterval = 30 * time.Second
			opts.NoticeInterval = -1
			stop := startServe(t, func(ctx context.Context) error {
				return c.Inbox().Serve(ctx, "a1", noopHandler, opts)
			})
			waitFor(t, "the first pull", func() bool { return pulls.count() >= 1 })
			stop()
			q := pulls.at(0).Query()
			if got := q.Get("wait"); got != tc.wait || q.Has("wait") != (tc.wait != "") {
				t.Errorf("wait = %q, want %q", got, tc.wait)
			}
			if got := q.Get("lease_secs"); got != tc.lease || q.Has("lease_secs") != (tc.lease != "") {
				t.Errorf("lease_secs = %q, want %q", got, tc.lease)
			}
			if q.Get("limit") != "25" || q.Get("agent_id") != "a1" {
				t.Errorf("query = %v", q)
			}
		})
	}
}

func TestServeWaitsNoLongerThanTheNextNoticesCheck(t *testing.T) {
	var pulls pullLog
	var notices atomic.Int32
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/inbox/notices":
			notices.Add(1)
			_, _ = w.Write([]byte(`[]`))
		case "/v1/inbox":
			pulls.add(r.URL)
			_, _ = w.Write([]byte(`[]`))
		default:
			http.NotFound(w, r)
		}
	})
	stop := startServe(t, func(ctx context.Context) error {
		return c.Inbox().Serve(ctx, "a1", noopHandler, ServeOptions{PollInterval: 30 * time.Second, NoticeInterval: 10 * time.Second})
	})
	waitFor(t, "the first pull", func() bool { return pulls.count() >= 1 })
	stop()
	if notices.Load() != 1 {
		t.Errorf("notices checks = %d, want 1 before the first pull", notices.Load())
	}
	// 10 s until the next check (rounded up), never the full 25.
	w, err := strconv.Atoi(pulls.at(0).Query().Get("wait"))
	if err != nil || w < 1 || w > 10 {
		t.Errorf("wait = %q, want 1-10", pulls.at(0).Query().Get("wait"))
	}
}

func TestServeCancelAbortsAWaitingPull(t *testing.T) {
	held := make(chan struct{}, 1)
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		select {
		case held <- struct{}{}:
		default:
		}
		// Hold the pull like a relay waiting for work.
		select {
		case <-r.Context().Done():
		case <-time.After(10 * time.Second):
		}
		w.Header().Set("ChakraMCP-Wait", "25")
		_, _ = w.Write([]byte(`[]`))
	})
	stop := startServe(t, func(ctx context.Context) error {
		return c.Inbox().Serve(ctx, "a1", noopHandler, ServeOptions{NoticeInterval: -1})
	})
	select {
	case <-held:
	case <-time.After(2 * time.Second):
		t.Fatal("no pull")
	}
	stop() // returns within 2 s though the relay would hold for 10
}

func TestServeReportsAClientTimeoutAndKeepsGoing(t *testing.T) {
	var pulls atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		pulls.Add(1)
		select {
		case <-r.Context().Done():
		case <-time.After(2 * time.Second):
		}
		_, _ = w.Write([]byte(`[]`))
	}))
	t.Cleanup(srv.Close)
	c, err := NewWithOptions(Options{
		APIKey:     "ck_test",
		RelayURL:   srv.URL,
		HTTPClient: &http.Client{Timeout: 50 * time.Millisecond},
	})
	if err != nil {
		t.Fatal(err)
	}
	var errs atomic.Int32
	stop := startServe(t, func(ctx context.Context) error {
		return c.Inbox().Serve(ctx, "a1", noopHandler, ServeOptions{
			PollInterval:   10 * time.Millisecond,
			NoticeInterval: -1,
			OnError:        func(error, *Invocation) { errs.Add(1) },
		})
	})
	// A timeout shorter than the wait is a failed pull, not the end.
	waitFor(t, "two reported timeouts", func() bool { return errs.Load() >= 2 })
	stop()
	if pulls.Load() < 2 {
		t.Errorf("pulls = %d, want at least 2", pulls.Load())
	}
}

func TestNoticeCarriesInvocationID(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte(`[{"id":"n1","kind":"reply_confirmed","created_at":"2026-10-29T00:00:00Z","invocation_id":"inv7"},` +
			`{"id":"n2","kind":"reply_confirmed","created_at":"2026-10-29T00:00:00Z"}]`))
	})
	notices, err := c.Inbox().Notices(context.Background(), "a1", 0)
	if err != nil {
		t.Fatalf("Notices: %v", err)
	}
	if len(notices) != 2 || notices[0].InvocationID == nil || *notices[0].InvocationID != "inv7" || notices[1].InvocationID != nil {
		t.Errorf("notices = %+v", notices)
	}
}

func TestReleasePostsAndReturnsTheInvocation(t *testing.T) {
	var method, path, body string
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		method, path = r.Method, r.URL.Path
		b, _ := io.ReadAll(r.Body)
		body = string(b)
		_, _ = w.Write([]byte(`{"id":"inv-1","status":"pending","deliveries":0,"lease_until":null}`))
	})
	inv, err := c.Inbox().Release(context.Background(), "inv-1")
	if err != nil {
		t.Fatalf("Release: %v", err)
	}
	if method != http.MethodPost || path != "/v1/invocations/inv-1/release" || body != "{}" {
		t.Errorf("sent %s %s %s", method, path, body)
	}
	if inv.Status != InvocationPending || inv.Deliveries != 0 || inv.LeaseUntil != nil {
		t.Errorf("got %+v", inv)
	}
}

func TestARefusedReleaseCarriesTheCode(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusConflict)
		_, _ = w.Write([]byte(`{"error":{"code":"chk.invocation.not_releasable","message":"only a claimed invocation can be released"}}`))
	})
	_, err := c.Inbox().Release(context.Background(), "inv-1")
	var apiErr *Error
	if !errors.As(err, &apiErr) || apiErr.Status != http.StatusConflict || apiErr.Code != "chk.invocation.not_releasable" {
		t.Fatalf("err = %v", err)
	}
}

func TestInvokeSendsExpiresInAndReadsPeer(t *testing.T) {
	var bodies []map[string]any
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		var b map[string]any
		_ = json.NewDecoder(r.Body).Decode(&b)
		bodies = append(bodies, b)
		w.WriteHeader(http.StatusAccepted)
		_, _ = w.Write([]byte(`{"invocation_id":"inv-1","status":"pending","error":null,
			"peer":{"delivery":"pull","online":false,"last_seen_at":"2026-10-29T09:12:03Z"}}`))
	})
	ctx := context.Background()
	res, err := c.Invoke(ctx, &InvokeRequest{GrantID: "g-1", GranteeAgentID: "a-1", Input: json.RawMessage(`{}`), ExpiresIn: 600})
	if err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if res.Peer == nil || res.Peer.Delivery != "pull" || res.Peer.Online || res.Peer.LastSeenAt == nil {
		t.Errorf("peer = %+v", res.Peer)
	}
	if _, err := c.Invoke(ctx, &InvokeRequest{GrantID: "g-1", GranteeAgentID: "a-1", Input: json.RawMessage(`{}`)}); err != nil {
		t.Fatalf("Invoke: %v", err)
	}
	if bodies[0]["expires_in"] != float64(600) {
		t.Errorf("first body = %v", bodies[0])
	}
	if _, set := bodies[1]["expires_in"]; set {
		t.Errorf("second body = %v", bodies[1])
	}
}

func TestPullReadsTheOwnersReplyLink(t *testing.T) {
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`[{"id":"inv1","capability_name":"message_owner","status":"in_progress","elapsed_ms":0,"input_preview":{},"created_at":"2026-01-01T00:00:00Z","i_served":true,"i_invoked":false,"semantics":"human_in_loop","owner_reply_url":"https://chakramcp.com/app/messages/inv1"}]`))
	})
	invs, err := c.Inbox().Pull(context.Background(), "a1", 0)
	if err != nil {
		t.Fatalf("Pull: %v", err)
	}
	if len(invs) != 1 || invs[0].OwnerReplyURL != "https://chakramcp.com/app/messages/inv1" {
		t.Errorf("got %+v", invs)
	}
}
