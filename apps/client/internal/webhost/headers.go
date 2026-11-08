package webhost

import (
	"net/http"
	"strconv"
	"strings"
)

// ContentSecurityPolicy matches the policy of the LitRadar API server. The Vite
// build ships no inline scripts, so script-src needs no hashes or nonces.
const ContentSecurityPolicy = "default-src 'self'; base-uri 'self'; object-src 'none'; " +
	"frame-ancestors 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"

const permissionsPolicy = "camera=(), microphone=(), geolocation=(), payment=(), usb=()"

// runtimePathPrefix is the namespace of the Wails runtime endpoints routed through
// the asset server: runtime calls, runtime.js, transport.js, event payloads, and
// stream polls.
const runtimePathPrefix = "/wails/"

// SecurityMiddleware sets the security headers on every asset-server response and
// answers 404 for the Wails runtime namespace. The web host binds no services, and
// an unauthenticated runtime call could otherwise reach built-in objects such as
// Application.Quit.
func SecurityMiddleware(config Config) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
			SetSecurityHeaders(writer.Header(), config.HstsMaxAge)
			if request.URL.Path == strings.TrimSuffix(runtimePathPrefix, "/") ||
				strings.HasPrefix(request.URL.Path, runtimePathPrefix) {
				http.NotFound(writer, request)
				return
			}
			next.ServeHTTP(writer, request)
		})
	}
}

// SetSecurityHeaders writes the browser hardening headers shared by every response.
// A positive hstsMaxAge also enables Strict-Transport-Security.
func SetSecurityHeaders(headers http.Header, hstsMaxAge int) {
	headers.Set("Content-Security-Policy", ContentSecurityPolicy)
	headers.Set("X-Content-Type-Options", "nosniff")
	headers.Set("Referrer-Policy", "same-origin")
	headers.Set("Permissions-Policy", permissionsPolicy)
	headers.Set("X-Frame-Options", "DENY")
	if hstsMaxAge > 0 {
		headers.Set("Strict-Transport-Security", "max-age="+strconv.Itoa(hstsMaxAge))
	}
}
