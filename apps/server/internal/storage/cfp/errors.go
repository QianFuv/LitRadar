package cfp

import "errors"

// PayloadError distinguishes rejected persisted/imported JSON from semantic validation.
type PayloadError struct{ Cause error }

func (failure *PayloadError) Error() string { return "CFP payload error: " + failure.Cause.Error() }
func (failure *PayloadError) Unwrap() error { return failure.Cause }

// DatabaseError retains the repository's public category without discarding the underlying cause.
type DatabaseError struct{ Cause error }

func (failure *DatabaseError) Error() string { return "CFP database error: " + failure.Cause.Error() }
func (failure *DatabaseError) Unwrap() error { return failure.Cause }

func databaseError(err error) error {
	if err == nil || errors.Is(err, ErrStaleRefresh) {
		return err
	}
	var invalidError *InvalidError
	var payloadError *PayloadError
	var storageError *DatabaseError
	if errors.As(err, &invalidError) || errors.As(err, &payloadError) || errors.As(err, &storageError) {
		return err
	}
	return &DatabaseError{Cause: err}
}
