package process

import "errors"

// SupervisorError preserves a stable phase without including OS messages or command arguments.
type SupervisorError struct {
	Kind  string
	cause error
}

func (failure *SupervisorError) Error() string    { return failure.Kind }
func (failure *SupervisorError) GoString() string { return failure.Kind }
func (failure *SupervisorError) Unwrap() error    { return failure.cause }

func classifiedError(kind string, err error) error {
	if err == nil {
		return nil
	}
	return &SupervisorError{Kind: kind, cause: err}
}

// ErrorKind returns a classified cleanup phase without parsing platform-dependent error text.
func ErrorKind(err error) string {
	var failure *SupervisorError
	if errors.As(err, &failure) {
		return failure.Kind
	}
	return "wait_failed"
}
