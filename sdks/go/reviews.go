package chakramcp

import (
	"context"
	"net/http"
	"net/url"
	"strconv"
)

// ReviewsClient covers the agent-to-agent ratings & reviews API
// (migration 0023). One review per (reviewer, target) pair; Write
// upserts in place.
type ReviewsClient struct {
	c *Client
}

// List fetches a target agent's reviews (cursor-paginated) plus a
// summary (average + per-bucket distribution). `IncludeHidden` is
// honoured server-side only when the caller is a member of the
// target's account; non-members get the flag silently cleared.
func (r *ReviewsClient) List(
	ctx context.Context,
	targetAgentID string,
	opts ReviewListQuery,
) (*ReviewListResponse, error) {
	q := url.Values{}
	if opts.Cursor != "" {
		q.Set("cursor", opts.Cursor)
	}
	if opts.Limit > 0 {
		q.Set("limit", strconv.Itoa(opts.Limit))
	}
	if opts.Tier != "" {
		q.Set("tier", string(opts.Tier))
	}
	if opts.IncludeHidden {
		q.Set("include_hidden", "true")
	}
	path := "/v1/agents/" + url.PathEscape(targetAgentID) + "/reviews"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out ReviewListResponse
	if err := r.c.do(ctx, http.MethodGet, r.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Write upserts a review for `targetAgentID`. Rating must be 1-5;
// `TaggedCapabilityIDs` must have length ≥ 1 and every entry must
// belong to the target AND have a non-rejected `relay_invocations`
// row from the reviewer.
func (r *ReviewsClient) Write(
	ctx context.Context,
	targetAgentID string,
	body *WriteReviewRequest,
) (*Review, error) {
	var out Review
	path := "/v1/agents/" + url.PathEscape(targetAgentID) + "/reviews"
	if err := r.c.do(ctx, http.MethodPost, r.c.relayURL, path, body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Eligibility returns the caller's agents (plus the target capabilities
// each has invoked) that are eligible to write a review. Agents the
// caller owns that haven't invoked anything on the target are filtered
// out — there's nothing tagable.
func (r *ReviewsClient) Eligibility(
	ctx context.Context,
	targetAgentID string,
) (*EligibilityResponse, error) {
	var out EligibilityResponse
	path := "/v1/agents/" + url.PathEscape(targetAgentID) + "/reviews/eligibility"
	if err := r.c.do(ctx, http.MethodGet, r.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Hide soft-hides a review on one of your agents. Caller must be a
// member of the target's account; non-members get 403.
func (r *ReviewsClient) Hide(
	ctx context.Context,
	targetAgentID, reviewID string,
) (*Review, error) {
	var out Review
	path := "/v1/agents/" + url.PathEscape(targetAgentID) +
		"/reviews/" + url.PathEscape(reviewID) + "/hide"
	if err := r.c.do(ctx, http.MethodPost, r.c.relayURL, path, struct{}{}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// Unhide reverses Hide.
func (r *ReviewsClient) Unhide(
	ctx context.Context,
	targetAgentID, reviewID string,
) (*Review, error) {
	var out Review
	path := "/v1/agents/" + url.PathEscape(targetAgentID) +
		"/reviews/" + url.PathEscape(reviewID) + "/unhide"
	if err := r.c.do(ctx, http.MethodPost, r.c.relayURL, path, struct{}{}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}
