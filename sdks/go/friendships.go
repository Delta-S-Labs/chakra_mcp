package chakramcp

import (
	"context"
	"net/http"
	"net/url"
)

type FriendshipsClient struct {
	c *Client
}

type ListFriendshipsOptions struct {
	Direction string           // "all" | "outbound" | "inbound"
	Status    FriendshipStatus // optional
}

func (f *FriendshipsClient) List(ctx context.Context, opts ListFriendshipsOptions) ([]Friendship, error) {
	q := url.Values{}
	if opts.Direction != "" {
		q.Set("direction", opts.Direction)
	}
	if opts.Status != "" {
		q.Set("status", string(opts.Status))
	}
	path := "/v1/friendships"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	var out []Friendship
	if err := f.c.do(ctx, http.MethodGet, f.c.relayURL, path, nil, &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (f *FriendshipsClient) Get(ctx context.Context, id string) (*Friendship, error) {
	var out Friendship
	if err := f.c.do(ctx, http.MethodGet, f.c.relayURL, "/v1/friendships/"+url.PathEscape(id), nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (f *FriendshipsClient) Propose(ctx context.Context, body *ProposeFriendshipRequest) (*Friendship, error) {
	var out Friendship
	if err := f.c.do(ctx, http.MethodPost, f.c.relayURL, "/v1/friendships", body, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

type respondBody struct {
	ResponseMessage *string `json:"response_message,omitempty"`
}
type counterBody struct {
	ProposerMessage string `json:"proposer_message"`
}

func (f *FriendshipsClient) Accept(ctx context.Context, id string, message *string) (*Friendship, error) {
	var out Friendship
	if err := f.c.do(ctx, http.MethodPost, f.c.relayURL, "/v1/friendships/"+url.PathEscape(id)+"/accept", &respondBody{ResponseMessage: message}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (f *FriendshipsClient) Reject(ctx context.Context, id string, message *string) (*Friendship, error) {
	var out Friendship
	if err := f.c.do(ctx, http.MethodPost, f.c.relayURL, "/v1/friendships/"+url.PathEscape(id)+"/reject", &respondBody{ResponseMessage: message}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (f *FriendshipsClient) Counter(ctx context.Context, id, message string) (*Friendship, error) {
	var out Friendship
	if err := f.c.do(ctx, http.MethodPost, f.c.relayURL, "/v1/friendships/"+url.PathEscape(id)+"/counter", &counterBody{ProposerMessage: message}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

func (f *FriendshipsClient) Cancel(ctx context.Context, id string) (*Friendship, error) {
	var out Friendship
	if err := f.c.do(ctx, http.MethodPost, f.c.relayURL, "/v1/friendships/"+url.PathEscape(id)+"/cancel", struct{}{}, &out); err != nil {
		return nil, err
	}
	return &out, nil
}
