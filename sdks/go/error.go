package chakramcp

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

// Error is returned for non-2xx API responses. Code and Message come
// from the standard {"error":{"code","message"}} envelope; raw body
// is preserved for cases where the body isn't a recognisable envelope.
type Error struct {
	Status  int
	Code    string
	Message string
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
// + ResetsAt and back off cleanly.
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

func errorFromResponse(resp *http.Response) error {
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()

	// Try the rich shape first: { error: {code,message}, quota, resets_at }.
	// Public-invoke monthly quota 429 is the only error today that uses it;
	// fall through to the generic Error when it doesn't match.
	if resp.StatusCode == http.StatusTooManyRequests {
		var richEnv struct {
			Error struct {
				Code    string `json:"code"`
				Message string `json:"message"`
			} `json:"error"`
			Quota    int32     `json:"quota"`
			ResetsAt time.Time `json:"resets_at"`
		}
		if err := json.Unmarshal(body, &richEnv); err == nil &&
			richEnv.Error.Code == "monthly_quota_exhausted" {
			return &QuotaExhaustedError{
				Err: &Error{
					Status:  resp.StatusCode,
					Code:    richEnv.Error.Code,
					Message: richEnv.Error.Message,
					Raw:     body,
				},
				Quota:    richEnv.Quota,
				ResetsAt: richEnv.ResetsAt,
			}
		}
	}

	var env struct {
		Error struct {
			Code    string `json:"code"`
			Message string `json:"message"`
		} `json:"error"`
	}
	if err := json.Unmarshal(body, &env); err == nil && env.Error.Code != "" {
		return &Error{
			Status:  resp.StatusCode,
			Code:    env.Error.Code,
			Message: env.Error.Message,
			Raw:     body,
		}
	}
	return &Error{
		Status:  resp.StatusCode,
		Code:    "unknown",
		Message: string(body),
		Raw:     body,
	}
}
