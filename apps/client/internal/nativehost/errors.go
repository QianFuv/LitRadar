package nativehost

// Stable error codes returned to the frontend in the cause of a Wails runtime error.
const (
	// CodeInvalidOrigin marks a server address that is not a valid origin.
	CodeInvalidOrigin = "invalid_origin"
	// CodeInsecureOrigin marks an http:// address for a non-loopback host.
	CodeInsecureOrigin = "insecure_origin"
	// CodeServerUnreachable marks a transport failure while contacting the server.
	CodeServerUnreachable = "server_unreachable"
	// CodeNotLitRadarServer marks an origin whose readiness probe did not answer like LitRadar.
	CodeNotLitRadarServer = "not_litradar_server"
	// CodeServerNotReady marks a LitRadar server whose readiness probe reported a failure.
	CodeServerNotReady = "server_not_ready"
	// CodeResponseTooLarge marks a response body above the client buffer limit.
	CodeResponseTooLarge = "response_too_large"
	// CodeServerNotConfigured marks a request made before a server origin is stored.
	CodeServerNotConfigured = "server_not_configured"
	// CodeForbiddenTarget marks a request outside the configured origin or API paths.
	CodeForbiddenTarget = "forbidden_target"
	// CodeInvalidRequest marks a malformed bridge request.
	CodeInvalidRequest = "invalid_request"
	// CodeInvalidToken marks a malformed access token.
	CodeInvalidToken = "invalid_token"
	// CodeStorageFailure marks a failure of the secure token store or the config file.
	CodeStorageFailure = "storage_failure"
	// CodeUnsupported marks an operation the current platform does not provide.
	CodeUnsupported = "unsupported"
	// CodeOidcError marks a single sign-on that did not complete; Reason names the cause.
	CodeOidcError = "oidc_error"
	// CodeOidcTimeout marks a single sign-on whose browser step did not finish in time.
	CodeOidcTimeout = "oidc_timeout"
)

// BridgeError is the error type returned by [Bridge] methods. Wails serializes it
// as the cause of the frontend runtime error, so the frontend can branch on Code.
type BridgeError struct {
	// Code is one of the Code* constants.
	Code string `json:"code"`
	// Message describes the failure for logs; it never contains secrets.
	Message string `json:"message"`
	// Reason is the bounded single sign-on failure code of a [CodeOidcError],
	// such as the oidc_error value the server reported.
	Reason string `json:"reason,omitempty"`
}

// Error implements [error].
func (bridgeError *BridgeError) Error() string {
	return bridgeError.Code + ": " + bridgeError.Message
}

// newBridgeError returns a [BridgeError] with the given code and message.
func newBridgeError(code, message string) *BridgeError {
	return &BridgeError{Code: code, Message: message}
}
