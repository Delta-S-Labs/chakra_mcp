package chakramcp

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
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
