package chakramcp

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
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

func errorFromResponse(resp *http.Response) error {
	body, _ := io.ReadAll(resp.Body)
	_ = resp.Body.Close()

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
