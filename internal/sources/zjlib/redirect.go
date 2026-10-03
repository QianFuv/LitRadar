package zjlib

import (
	"context"
	"errors"
	"net/http"

	"github.com/QianFuv/LitRadar/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func (live *LiveTransport) roundTripSequence(ctx context.Context, method, location, body string, headers http.Header, shouldRedirect bool) (*http.Response, error) {
	initial, err := whatwg.NewParser().Parse(location)
	if err != nil {
		return nil, &Error{Kind: "Request", Message: "builder error"}
	}
	var policy func(string, int) error
	if shouldRedirect {
		policy = func(next string, previousCount int) error {
			family, err := live.state.allowed.family(initial)
			if err == nil {
				_, err = live.state.allowed.parse(next, family)
			}
			if previousCount > 10 || err != nil {
				return &Error{Kind: "Request", Message: "error following redirect"}
			}
			return nil
		}
	}
	response, err := transport.RedirectSequence(ctx, live.state.redirect.Transport, live.state.redirect.Jar, live.state.proxy, method, location, body, headers, policy)
	if err != nil {
		var failure *Error
		if errors.As(err, &failure) {
			return nil, failure
		}
		if errors.Is(err, transport.ErrRequestBuild) {
			return nil, &Error{Kind: "Request", Message: "builder error"}
		}
		return nil, &Error{Kind: "Request", Message: "error sending request"}
	}
	return response, nil
}
