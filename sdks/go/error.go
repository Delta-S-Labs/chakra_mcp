package chakramcp

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"
)

// Error is returned for non-2xx API responses. Code and Message come
// from the standard {"error":{"code","message"}} envelope; raw body
// is preserved for cases where the body isn't a recognisable envelope.
type Error struct {
	Status  int
	Code    string
	Message string
	// RetryAfterSecs is how long to wait before retrying, in seconds,
	// when the server says: the Retry-After header, else the body's
	// retry_after_secs. Nil when it says neither (most errors, and
	// older servers).
	RetryAfterSecs *int64
	// Raw is the full response body. Useful when the server returns
	// something other than the standard envelope (e.g. a stray HTML
	// page from a misconfigured proxy).
	Raw []byte
}

func (e *Error) Error() string {
	return fmt.Sprintf("chakramcp: [%d %s] %s", e.Status, e.Code, e.Message)
}

// QuotaExhaustedError is returned when a public-invoke call exceeds
// the per-invoker monthly quota on the target capability (HTTP 429 +
// body code "monthly_quota_exhausted", migration 0022). Wraps the
// generic Error via Unwrap so existing
// `errors.As(err, &chakramcp.Error{})` callers still match; new
// callers can switch on
// `errors.As(err, &chakramcp.QuotaExhaustedError{})` to read Quota
// + ResetsAt and back off cleanly. Err.RetryAfterSecs carries the
// Retry-After delay.
type QuotaExhaustedError struct {
	Err      *Error
	Quota    int32
	ResetsAt time.Time
}

func (e *QuotaExhaustedError) Error() string {
	return fmt.Sprintf(
		"chakramcp: [429 monthly_quota_exhausted] %s (quota=%d, resets_at=%s)",
		e.Err.Message, e.Quota, e.ResetsAt.Format(time.RFC3339),
	)
}

func (e *QuotaExhaustedError) Unwrap() error { return e.Err }

// RateLimitedError is returned when the calling account is over its
// per-minute rate limit (HTTP 429 + body code "account_rate_limited").
// Err.RetryAfterSecs is the rest of the minute (nil on older servers).
// Wraps the generic Error via Unwrap, like QuotaExhaustedError. The
// SDK doesn't retry on its own.
type RateLimitedError struct {
	Err *Error
}

func (e *RateLimitedError) Error() string {
	return fmt.Sprintf(
		"chakramcp: [429 account_rate_limited] %s%s",
		e.Err.Message, retryAfterSuffix(e.Err.RetryAfterSecs),
	)
}

func (e *RateLimitedError) Unwrap() error { return e.Err }

// CreditsExhaustedError is returned when the calling account is out of
// credits (HTTP 429 + body code "account_credits_exhausted").
// Err.RetryAfterSecs runs until the next free monthly grant and can be
// weeks long; buying credits works at once. Wraps the generic Error
// via Unwrap, like QuotaExhaustedError.
type CreditsExhaustedError struct {
	Err *Error
	// NextGrantAt is when the next free monthly grant lands. Zero on
	// older servers.
	NextGrantAt time.Time
	// CreditsURL is where to buy credits. Empty on self-hosted servers
	// and older ones.
	CreditsURL string
}

func (e *CreditsExhaustedError) Error() string {
	msg := fmt.Sprintf(
		"chakramcp: [429 account_credits_exhausted] %s%s",
		e.Err.Message, retryAfterSuffix(e.Err.RetryAfterSecs),
	)
	if !e.NextGrantAt.IsZero() {
		msg += " (next_grant_at=" + e.NextGrantAt.Format(time.RFC3339) + ")"
	}
	if e.CreditsURL != "" {
		msg += " (credits_url=" + e.CreditsURL + ")"
	}
	return msg
}

func (e *CreditsExhaustedError) Unwrap() error { return e.Err }

func retryAfterSuffix(secs *int64) string {
	if secs == nil {
		return ""
	}
	return fmt.Sprintf(" (retry_after=%ds)", *secs)
}

// retryAfterSecs is the Retry-After header when it's a whole number of
// seconds, else the body's retry_after_secs, else nil.
func retryAfterSecs(h http.Header, fromBody *int64) *int64 {
	if v := strings.TrimSpace(h.Get("Retry-After")); v != "" {
		if n, err := strconv.ParseInt(v, 10, 64); err == nil && n >= 0 {
			return &n
		}
	}
	if fromBody != nil && *fromBody >= 0 {
		return fromBody
	}
	return nil
}

func errorFromResponse(resp *http.Response) error {
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()

	// Try the rich shape first: { error: {code,message}, quota, resets_at }.
	// Public-invoke monthly quota 429 is the only error today that uses it;
	// fall through to the generic Error when it doesn't match.
	if resp.StatusCode == http.StatusTooManyRequests {
		var richEnv struct {
			Error struct {
				Code           string `json:"code"`
				Message        string `json:"message"`
				RetryAfterSecs *int64 `json:"retry_after_secs"`
			} `json:"error"`
			Quota    int32     `json:"quota"`
			ResetsAt time.Time `json:"resets_at"`
		}
		if err := json.Unmarshal(body, &richEnv); err == nil &&
			richEnv.Error.Code == "monthly_quota_exhausted" {
			return &QuotaExhaustedError{
				Err: &Error{
					Status:         resp.StatusCode,
					Code:           richEnv.Error.Code,
					Message:        richEnv.Error.Message,
					RetryAfterSecs: retryAfterSecs(resp.Header, richEnv.Error.RetryAfterSecs),
					Raw:            body,
				},
				Quota:    richEnv.Quota,
				ResetsAt: richEnv.ResetsAt,
			}
		}
	}

	// The standard envelope. retry_after_secs, next_grant_at and
	// credits_url are only on usage-limit 429s from newer servers.
	var env struct {
		Error struct {
			Code           string `json:"code"`
			Message        string `json:"message"`
			RetryAfterSecs *int64 `json:"retry_after_secs"`
			NextGrantAt    string `json:"next_grant_at"`
			CreditsURL     string `json:"credits_url"`
		} `json:"error"`
	}
	if err := json.Unmarshal(body, &env); err == nil && env.Error.Code != "" {
		apiErr := &Error{
			Status:         resp.StatusCode,
			Code:           env.Error.Code,
			Message:        env.Error.Message,
			RetryAfterSecs: retryAfterSecs(resp.Header, env.Error.RetryAfterSecs),
			Raw:            body,
		}
		if resp.StatusCode == http.StatusTooManyRequests {
			switch env.Error.Code {
			case "account_rate_limited":
				return &RateLimitedError{Err: apiErr}
			case "account_credits_exhausted":
				// An unparseable time is left zero rather than failing
				// the whole error.
				nextGrantAt, _ := time.Parse(time.RFC3339, env.Error.NextGrantAt)
				return &CreditsExhaustedError{
					Err:         apiErr,
					NextGrantAt: nextGrantAt,
					CreditsURL:  env.Error.CreditsURL,
				}
			}
		}
		return apiErr
	}
	return &Error{
		Status:         resp.StatusCode,
		Code:           "unknown",
		Message:        string(body),
		RetryAfterSecs: retryAfterSecs(resp.Header, nil),
		Raw:            body,
	}
}
