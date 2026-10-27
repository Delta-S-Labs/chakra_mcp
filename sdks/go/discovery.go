package chakramcp

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"strings"
)

// DiscoveryClient covers the public directory (relays from 0.6.0):
// ranked search, featured agents, profiles and your standing. Results
// include your own and your orgs' agents, and your standing (You).
type DiscoveryClient struct {
	c *Client
}

// agentRefPath is an agent's id, or `account_slug/agent_slug`, as a URL
// path.
func agentRefPath(ref string) string {
	if i := strings.IndexByte(ref, '/'); i >= 0 {
		return url.PathEscape(ref[:i]) + "/" + url.PathEscape(ref[i+1:])
	}
	return url.PathEscape(ref)
}

// Search searches the directory. For the next page, pass
// DiscoveryPage.NextCursor back as Cursor, with the same filters.
func (d *DiscoveryClient) Search(ctx context.Context, query DiscoveryQuery) (*DiscoveryPage, error) {
	q := url.Values{}
	if query.Q != "" {
		q.Set("q", query.Q)
	}
	if query.Capability != "" {
		q.Set("capability", query.Capability)
	}
	if query.CapabilitySchema != nil {
		schema, err := json.Marshal(query.CapabilitySchema)
		if err != nil {
			return nil, fmt.Errorf("chakramcp: marshal capability_schema: %w", err)
		}
		q.Set("capability_schema", string(schema))
	}
	if query.Mode != "" {
		q.Set("mode", string(query.Mode))
	}
	if query.Verified != nil {
		q.Set("verified", strconv.FormatBool(*query.Verified))
	}
	if len(query.Tags) > 0 {
		q.Set("tags", strings.Join(query.Tags, ","))
	}
	if query.IncludeDormant {
		q.Set("include_dormant", "true")
	}
	if query.Callable {
		q.Set("callable", "true")
	}
	if query.Limit > 0 {
		q.Set("limit", strconv.Itoa(query.Limit))
	}
	if query.Cursor != "" {
		q.Set("cursor", query.Cursor)
	}
	path := "/v1/discovery/agents"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out DiscoveryPage
	if err := d.c.do(ctx, http.MethodGet, d.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Featured returns the agents worth trying first: the relay's house
// agents, then online ones with something public.
func (d *DiscoveryClient) Featured(ctx context.Context) ([]FeaturedAgent, error) {
	var out struct {
		Agents []FeaturedAgent `json:"agents"`
	}
	if err := d.c.do(ctx, http.MethodGet, d.c.relayURL, "/v1/discovery/featured", nil, &out); err != nil {
		return nil, err
	}
	return out.Agents, nil
}

// Profile returns one agent's profile, by id or `account_slug/agent_slug`.
func (d *DiscoveryClient) Profile(ctx context.Context, ref string) (*AgentProfile, error) {
	var out AgentProfile
	if err := d.c.do(ctx, http.MethodGet, d.c.relayURL, "/v1/discovery/agents/"+agentRefPath(ref), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Standing returns your standing with one agent, by id or
// `account_slug/agent_slug`.
func (d *DiscoveryClient) Standing(ctx context.Context, ref string) (*Standing, error) {
	var out Standing
	path := "/v1/discovery/standing?" + url.Values{"agent": {ref}}.Encode()
	if err := d.c.do(ctx, http.MethodGet, d.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}
