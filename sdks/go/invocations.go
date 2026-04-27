package chakramcp

import (
	"context"
	"net/http"
	"net/url"
)

type InvocationsClient struct {
	c *Client
}

type ListInvocationsOptions struct {
	Direction string           // "all" | "outbound" | "inbound"
	AgentID   string           // filter by either side
	Status    InvocationStatus // optional
}

func (i *InvocationsClient) List(ctx context.Context, opts ListInvocationsOptions) ([]Invocation, error) {
	q := url.Values{}
	if opts.Direction != "" {
		q.Set("direction", opts.Direction)
	}
	if opts.AgentID != "" {
		q.Set("agent_id", opts.AgentID)
	}
	if opts.Status != "" {
		q.Set("status", string(opts.Status))
	}
	path := "/v1/invocations"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out []Invocation
	if err := i.c.do(ctx, http.MethodGet, i.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (i *InvocationsClient) Get(ctx context.Context, id string) (*Invocation, error) {
	var out Invocation
	if err := i.c.do(ctx, http.MethodGet, i.c.relayURL, "/v1/invocations/"+url.PathEscape(id), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}
