package chakramcp

import (
	"context"
	"net/http"
	"net/url"
)

type AgentsClient struct {
	c *Client
}

func (a *AgentsClient) Capabilities() *CapabilitiesClient {
	return &CapabilitiesClient{c: a.c}
}

func (a *AgentsClient) List(ctx context.Context) ([]Agent, error) {
	var out []Agent
	if err := a.c.do(ctx, http.MethodGet, a.c.relayURL, "/v1/agents", nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (a *AgentsClient) Get(ctx context.Context, id string) (*Agent, error) {
	var out Agent
	if err := a.c.do(ctx, http.MethodGet, a.c.relayURL, "/v1/agents/"+url.PathEscape(id), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (a *AgentsClient) Create(ctx context.Context, body *CreateAgentRequest) (*Agent, error) {
	var out Agent
	if err := a.c.do(ctx, http.MethodPost, a.c.relayURL, "/v1/agents", body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (a *AgentsClient) Update(ctx context.Context, id string, body *UpdateAgentRequest) (*Agent, error) {
	var out Agent
	if err := a.c.do(ctx, http.MethodPatch, a.c.relayURL, "/v1/agents/"+url.PathEscape(id), body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (a *AgentsClient) Delete(ctx context.Context, id string) error {
	return a.c.do(ctx, http.MethodDelete, a.c.relayURL, "/v1/agents/"+url.PathEscape(id), nil, nil)
}

type CapabilitiesClient struct {
	c *Client
}

func (cc *CapabilitiesClient) List(ctx context.Context, agentID string) ([]Capability, error) {
	var out []Capability
	if err := cc.c.do(ctx, http.MethodGet, cc.c.relayURL, "/v1/agents/"+url.PathEscape(agentID)+"/capabilities", nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (cc *CapabilitiesClient) Create(ctx context.Context, agentID string, body *CreateCapabilityRequest) (*Capability, error) {
	var out Capability
	if err := cc.c.do(ctx, http.MethodPost, cc.c.relayURL, "/v1/agents/"+url.PathEscape(agentID)+"/capabilities", body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (cc *CapabilitiesClient) Delete(ctx context.Context, agentID, capabilityID string) error {
	return cc.c.do(ctx, http.MethodDelete, cc.c.relayURL, "/v1/agents/"+url.PathEscape(agentID)+"/capabilities/"+url.PathEscape(capabilityID), nil, nil)
}
