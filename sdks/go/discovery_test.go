package chakramcp

import (
	"context"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// discoveryFixture reads a response recorded from the relay, shared with
// the other SDKs (sdks/testdata/discovery).
func discoveryFixture(t *testing.T, name string) []byte {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("..", "testdata", "discovery", name+".json"))
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	return b
}

// serveFixture answers every request with a fixture and records each
// request as "METHOD /path?query", escaped as sent.
func serveFixture(t *testing.T, name string) (*Client, *[]string) {
	t.Helper()
	body := discoveryFixture(t, name)
	seen := &[]string{}
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		*seen = append(*seen, r.Method+" "+r.RequestURI)
		w.Header().Set("content-type", "application/json")
		_, _ = w.Write(body)
	})
	return c, seen
}

func TestDiscoverySearchSendsFiltersAndDecodesPage(t *testing.T) {
	var query map[string]string
	var path string
	body := discoveryFixture(t, "search")
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		path = r.URL.Path
		query = map[string]string{}
		for k := range r.URL.Query() {
			query[k] = r.URL.Query().Get(k)
		}
		_, _ = w.Write(body)
	})
	unverified := false
	page, err := c.Discovery().Search(context.Background(), DiscoveryQuery{
		Q:                "guide",
		Tags:             []string{"help", "docs"},
		Callable:         true,
		Verified:         &unverified,
		CapabilitySchema: map[string]interface{}{"properties": map[string]interface{}{"answer": map[string]interface{}{}}},
		Limit:            2,
	})
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if path != "/v1/discovery/agents" {
		t.Errorf("path = %q", path)
	}
	var schema any
	if err := json.Unmarshal([]byte(query["capability_schema"]), &schema); err != nil {
		t.Errorf("capability_schema %q: %v", query["capability_schema"], err)
	}
	if want := map[string]any{"properties": map[string]any{"answer": map[string]any{}}}; !reflect.DeepEqual(schema, want) {
		t.Errorf("capability_schema = %v, want %v", schema, want)
	}
	delete(query, "capability_schema")
	want := map[string]string{"q": "guide", "tags": "help,docs", "callable": "true", "verified": "false", "limit": "2"}
	if !reflect.DeepEqual(query, want) {
		t.Errorf("query = %v, want %v", query, want)
	}

	if page.TotalEstimate == nil || *page.TotalEstimate != 1 || page.NextCursor != nil {
		t.Errorf("total_estimate/next_cursor = %v/%v, want 1/nil", page.TotalEstimate, page.NextCursor)
	}
	if len(page.Agents) != 1 {
		t.Fatalf("agents = %+v, want 1", page.Agents)
	}
	guide := page.Agents[0]
	if guide.Slug != "guide" || guide.AccountSlug != "chakramcp" || guide.Mode != AgentModePull ||
		guide.Visibility != VisibilityNetwork || !guide.Featured || guide.AcceptCalls != AcceptCallsFriends {
		t.Errorf("agent = %+v", guide.AgentListing)
	}
	if guide.LastSeenAt.IsZero() || guide.CreatedAt.IsZero() || guide.AvgRating != nil {
		t.Errorf("last_seen_at/created_at/avg_rating = %v/%v/%v", guide.LastSeenAt, guide.CreatedAt, guide.AvgRating)
	}
	if len(guide.Capabilities) != 1 || guide.Capabilities[0].Name != "guide.ask" || guide.Capabilities[0].Access != AccessPublic {
		t.Errorf("capabilities = %+v", guide.Capabilities)
	}
	if guide.AgentSlug != "guide" || !guide.HasPublicCapabilities || guide.You != nil {
		t.Errorf("agent_slug/has_public_capabilities/you = %q/%v/%v", guide.AgentSlug, guide.HasPublicCapabilities, guide.You)
	}
}

func TestDiscoverySearchWithoutFiltersSendsNoQuery(t *testing.T) {
	c, seen := serveFixture(t, "search")
	if _, err := c.Discovery().Search(context.Background(), DiscoveryQuery{}); err != nil {
		t.Fatalf("Search: %v", err)
	}
	if want := []string{"GET /v1/discovery/agents"}; !reflect.DeepEqual(*seen, want) {
		t.Errorf("requests = %v, want %v", *seen, want)
	}
}

func TestDiscoveryFeaturedReturnsTheAgents(t *testing.T) {
	c, seen := serveFixture(t, "featured")
	agents, err := c.Discovery().Featured(context.Background())
	if err != nil {
		t.Fatalf("Featured: %v", err)
	}
	if want := []string{"GET /v1/discovery/featured"}; !reflect.DeepEqual(*seen, want) {
		t.Errorf("requests = %v, want %v", *seen, want)
	}
	if len(agents) != 2 {
		t.Fatalf("agents = %+v, want 2", agents)
	}
	if !agents[0].Featured || agents[0].Slug != "guide" || agents[1].Slug != "ping" {
		t.Errorf("agents = %+v / %+v", agents[0].AgentListing, agents[1].AgentListing)
	}
	if len(agents[1].PublicCapabilities) != 1 || agents[1].PublicCapabilities[0].Name != "ping.echo" ||
		agents[1].PublicCapabilities[0].Description == "" {
		t.Errorf("public_capabilities = %+v", agents[1].PublicCapabilities)
	}
}

func TestDiscoveryProfileTakesAnIDOrAccountAndAgent(t *testing.T) {
	c, seen := serveFixture(t, "profile")
	ctx := context.Background()
	profile, err := c.Discovery().Profile(ctx, "chakramcp/guide")
	if err != nil {
		t.Fatalf("Profile: %v", err)
	}
	for _, ref := range []string{"01a1607c-5ca5-727e-96b8-4fcc3b4a18f7", "acme co/guide/v2"} {
		if _, err := c.Discovery().Profile(ctx, ref); err != nil {
			t.Fatalf("Profile(%q): %v", ref, err)
		}
	}
	want := []string{
		"GET /v1/discovery/agents/chakramcp/guide",
		"GET /v1/discovery/agents/01a1607c-5ca5-727e-96b8-4fcc3b4a18f7",
		// Split at the first slash; each part is escaped.
		"GET /v1/discovery/agents/acme%20co/guide%2Fv2",
	}
	if !reflect.DeepEqual(*seen, want) {
		t.Errorf("requests = %v, want %v", *seen, want)
	}

	if profile.Slug != "guide" || profile.Owner.AccountSlug != "chakramcp" || profile.Owner.DisplayName != "ChakraMCP" {
		t.Errorf("profile = %+v, owner = %+v", profile.AgentListing, profile.Owner)
	}
	if profile.AvgRating == nil || *profile.AvgRating != 5 || profile.ReviewCount != 1 {
		t.Errorf("avg_rating/review_count = %v/%d", profile.AvgRating, profile.ReviewCount)
	}
	if len(profile.Capabilities) != 1 {
		t.Fatalf("capabilities = %+v, want 1", profile.Capabilities)
	}
	cp := profile.Capabilities[0]
	if cp.Access != AccessPublic || cp.InputSchema["type"] != "object" || cp.OutputSchema["type"] != "object" ||
		cp.PublicMonthlyQuotaPerAgent == nil || *cp.PublicMonthlyQuotaPerAgent != 300 ||
		cp.ConsentRequired || cp.Semantics != SemanticsAutonomous {
		t.Errorf("capability = %+v", cp)
	}
	r := profile.Reviews
	if r.AvgRating == nil || *r.AvgRating != 5 || r.Count != 1 || len(r.Latest) != 1 {
		t.Fatalf("reviews = %+v", r)
	}
	if l := r.Latest[0]; l.Rating != 5 || l.Text == nil || *l.Text != "Clear answers, fast." || l.By != "An agent" || l.At.IsZero() {
		t.Errorf("latest review = %+v", l)
	}
	h := profile.HowToCall
	if !strings.Contains(h.CLI, "chakramcp invoke ensure chakramcp/guide") || h.A2ACard == "" || h.Profile == "" || h.MCP != "" {
		t.Errorf("how_to_call = %+v", h)
	}
	if profile.You != nil {
		t.Errorf("you = %+v, want nil", profile.You)
	}
}

func TestDiscoveryStandingNamesTheAgentInTheQuery(t *testing.T) {
	c, seen := serveFixture(t, "standing")
	you, err := c.Discovery().Standing(context.Background(), "bob/venue-bot")
	if err != nil {
		t.Fatalf("Standing: %v", err)
	}
	if want := []string{"GET /v1/discovery/standing?agent=bob%2Fvenue-bot"}; !reflect.DeepEqual(*seen, want) {
		t.Errorf("requests = %v, want %v", *seen, want)
	}
	if len(you.Callable) != 2 {
		t.Fatalf("callable = %+v, want 2", you.Callable)
	}
	grant, public := you.Callable[0], you.Callable[1]
	if grant.Via != CallableViaGrant || grant.Capability != "hold_room" || grant.AsAgentID == nil ||
		*grant.AsAgentID != "019a2b3c-0000-7000-8000-0000000000a1" {
		t.Errorf("callable[0] = %+v", grant)
	}
	if public.Via != CallableViaPublic || public.AsAgentID != nil {
		t.Errorf("callable[1] = %+v", public)
	}
	if len(you.Friendships) != 1 || you.Friendships[0].Status != FriendshipAccepted ||
		you.Friendships[0].YourAgentID != "019a2b3c-0000-7000-8000-0000000000a1" {
		t.Errorf("friendships = %+v", you.Friendships)
	}
	if len(you.CanStartCallAs) != 1 {
		t.Errorf("can_start_call_as = %v, want 1", you.CanStartCallAs)
	}
}

func TestAgentsCreateSendsTags(t *testing.T) {
	var bodies []map[string]any
	_, c := newServer(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/v1/agents" {
			http.NotFound(w, r)
			return
		}
		bodies = append(bodies, readBody(t, r))
		_, _ = w.Write([]byte(`{"id":"a1","slug":"rooms","display_name":"Rooms","tags":["booking"]}`))
	})
	ctx := context.Background()
	agent, err := c.Agents().Create(ctx, &CreateAgentRequest{
		AccountID: "acc1", Slug: "rooms", DisplayName: "Rooms", Tags: []string{"booking"},
	})
	if err != nil {
		t.Fatalf("Create: %v", err)
	}
	if got := bodies[0]["tags"]; !reflect.DeepEqual(got, []any{"booking"}) {
		t.Errorf("sent tags = %v, want [booking]", got)
	}
	if !reflect.DeepEqual(agent.Tags, []string{"booking"}) {
		t.Errorf("agent.Tags = %v, want [booking]", agent.Tags)
	}

	// No tags: none sent.
	if _, err := c.Agents().Create(ctx, &CreateAgentRequest{AccountID: "acc1", Slug: "rooms", DisplayName: "Rooms"}); err != nil {
		t.Fatalf("Create: %v", err)
	}
	if _, ok := bodies[1]["tags"]; ok {
		t.Errorf("tags sent though unset: %v", bodies[1])
	}
}
