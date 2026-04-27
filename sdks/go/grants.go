package chakramcp

import (
	"context"
	"net/http"
	"net/url"
)

type GrantsClient struct {
	c *Client
}

type ListGrantsOptions struct {
	Direction string      // "all" | "outbound" | "inbound"
	Status    GrantStatus // optional
}

func (g *GrantsClient) List(ctx context.Context, opts ListGrantsOptions) ([]Grant, error) {
	q := url.Values{}
	if opts.Direction != "" {
		q.Set("direction", opts.Direction)
	}
	if opts.Status != "" {
		q.Set("status", string(opts.Status))
	}
	path := "/v1/grants"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out []Grant
	if err := g.c.do(ctx, http.MethodGet, g.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (g *GrantsClient) Get(ctx context.Context, id string) (*Grant, error) {
	var out Grant
	if err := g.c.do(ctx, http.MethodGet, g.c.relayURL, "/v1/grants/"+url.PathEscape(id), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (g *GrantsClient) Create(ctx context.Context, body *CreateGrantRequest) (*Grant, error) {
	var out Grant
	if err := g.c.do(ctx, http.MethodPost, g.c.relayURL, "/v1/grants", body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

type revokeBody struct {
	Reason *string `json:"reason,omitempty"`
}

func (g *GrantsClient) Revoke(ctx context.Context, id string, reason *string) (*Grant, error) {
	var out Grant
	if err := g.c.do(ctx, http.MethodPost, g.c.relayURL, "/v1/grants/"+url.PathEscape(id)+"/revoke", &revokeBody{Reason: reason}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}
