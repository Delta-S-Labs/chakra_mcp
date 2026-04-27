// Package chakramcp is the Go SDK for the ChakraMCP relay.
//
// API-key auth only — for OAuth, use the CLI (`chakramcp login`).
// Uses context.Context throughout for cancellation.
//
// Quick start:
//
//	chakra, err := chakramcp.New(os.Getenv("CHAKRAMCP_API_KEY"))
//	if err != nil { log.Fatal(err) }
//	me, err := chakra.Me(ctx)
package chakramcp

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const (
	defaultAppURL    = "https://chakramcp.com"
	defaultRelayURL  = "https://relay.chakramcp.com"
	defaultUserAgent = "chakramcp-go-sdk"
	defaultTimeout   = 60 * time.Second
)

// Client is the top-level handle. Cheap to share across goroutines —
// the embedded *http.Client handles its own connection pooling.
type Client struct {
	apiKey   string
	appURL   string
	relayURL string
	http     *http.Client
}

// Options is the long form of New for callers who want to override URLs
// or supply a custom *http.Client (for testing, or to reuse a shared
// transport with proxy settings, etc.).
type Options struct {
	APIKey     string
	AppURL     string
	RelayURL   string
	HTTPClient *http.Client
}

// New constructs a Client pointing at the hosted public network with
// default options. Use NewWithOptions to override.
func New(apiKey string) (*Client, error) {
	return NewWithOptions(Options{APIKey: apiKey})
}

// NewWithOptions builds a Client. Empty string fields fall back to the
// hosted-network defaults; the *http.Client falls back to a fresh one
// with a 60s timeout.
func NewWithOptions(opts Options) (*Client, error) {
	if !strings.HasPrefix(opts.APIKey, "ck_") {
		return nil, errors.New("chakramcp: api_key must be a `ck_…` API key")
	}
	app := strings.TrimRight(orDefault(opts.AppURL, defaultAppURL), "/")
	relay := strings.TrimRight(orDefault(opts.RelayURL, defaultRelayURL), "/")
	if _, err := url.Parse(app); err != nil {
		return nil, fmt.Errorf("chakramcp: invalid app URL: %w", err)
	}
	if _, err := url.Parse(relay); err != nil {
		return nil, fmt.Errorf("chakramcp: invalid relay URL: %w", err)
	}
	httpc := opts.HTTPClient
	if httpc == nil {
		httpc = &http.Client{Timeout: defaultTimeout}
	}
	return &Client{
		apiKey:   opts.APIKey,
		appURL:   app,
		relayURL: relay,
		http:     httpc,
	}, nil
}

// AppURL returns the configured user-facing API URL.
func (c *Client) AppURL() string { return c.appURL }

// RelayURL returns the configured relay URL.
func (c *Client) RelayURL() string { return c.relayURL }

// Sub-client constructors. Free to call repeatedly.
func (c *Client) Agents() *AgentsClient           { return &AgentsClient{c: c} }
func (c *Client) Friendships() *FriendshipsClient { return &FriendshipsClient{c: c} }
func (c *Client) Grants() *GrantsClient           { return &GrantsClient{c: c} }
func (c *Client) Invocations() *InvocationsClient { return &InvocationsClient{c: c} }
func (c *Client) Inbox() *InboxClient             { return &InboxClient{c: c} }

// ─── Top-level RPCs ──────────────────────────────────────

func (c *Client) Me(ctx context.Context) (*MeResponse, error) {
	var out MeResponse
	if err := c.do(ctx, http.MethodGet, c.appURL, "/v1/me", nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (c *Client) Network(ctx context.Context) ([]Agent, error) {
	var out []Agent
	if err := c.do(ctx, http.MethodGet, c.relayURL, "/v1/network/agents", nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

// Invoke enqueues an invocation and returns the InvokeResponse.
// Use InvokeAndWait to also poll until the invocation reaches a
// terminal status.
func (c *Client) Invoke(ctx context.Context, req *InvokeRequest) (*InvokeResponse, error) {
	var out InvokeResponse
	if err := c.do(ctx, http.MethodPost, c.relayURL, "/v1/invoke", req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// PollOptions configures InvokeAndWait. Zero values fall back to
// Interval=1500ms, Timeout=3min.
type PollOptions struct {
	Interval time.Duration
	Timeout  time.Duration
}

// InvokeAndWait enqueues an invocation and polls until it reaches a
// terminal status. Returns context.DeadlineExceeded (wrapped) if
// Timeout elapses; the invocation may still be in flight.
func (c *Client) InvokeAndWait(ctx context.Context, req *InvokeRequest, opts PollOptions) (*Invocation, error) {
	interval := opts.Interval
	if interval == 0 {
		interval = 1500 * time.Millisecond
	}
	timeout := opts.Timeout
	if timeout == 0 {
		timeout = 3 * time.Minute
	}

	enq, err := c.Invoke(ctx, req)
	if err != nil {
		return nil, err
	}
	if enq.Status.IsTerminal() {
		return c.Invocations().Get(ctx, enq.InvocationID)
	}

	deadline := time.Now().Add(timeout)
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-ticker.C:
			fresh, err := c.Invocations().Get(ctx, enq.InvocationID)
			if err != nil {
				return nil, err
			}
			if fresh.Status.IsTerminal() {
				return fresh, nil
			}
			if time.Now().After(deadline) {
				return nil, fmt.Errorf("chakramcp: invoke_and_wait timed out after %s — invocation %s still in flight", timeout, enq.InvocationID)
			}
		}
	}
}

// ─── HTTP plumbing ───────────────────────────────────────

func (c *Client) do(ctx context.Context, method, base, path string, body any, out any) error {
	var reader io.Reader
	if body != nil {
		buf, err := json.Marshal(body)
		if err != nil {
			return fmt.Errorf("chakramcp: marshal body: %w", err)
		}
		reader = bytes.NewReader(buf)
	}
	req, err := http.NewRequestWithContext(ctx, method, base+path, reader)
	if err != nil {
		return err
	}
	req.Header.Set("authorization", "Bearer "+c.apiKey)
	req.Header.Set("user-agent", defaultUserAgent)
	if body != nil {
		req.Header.Set("content-type", "application/json")
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	if resp.StatusCode == http.StatusNoContent {
		_ = resp.Body.Close()
		return nil
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return errorFromResponse(resp)
	}
	defer resp.Body.Close()
	if out == nil {
		_, _ = io.Copy(io.Discard, resp.Body)
		return nil
	}
	dec := json.NewDecoder(resp.Body)
	if err := dec.Decode(out); err != nil {
		return fmt.Errorf("chakramcp: decode response: %w", err)
	}
	return nil
}

func orDefault(s, def string) string {
	if s == "" {
		return def
	}
	return s
}
