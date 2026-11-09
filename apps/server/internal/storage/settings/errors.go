package settings

// ErrorKind preserves business rejection categories for later HTTP and audit mapping.
type ErrorKind string

const (
	InvalidSetting       ErrorKind = "invalid_setting"
	UnknownSetting       ErrorKind = "unknown_setting"
	InvalidSecretPool    ErrorKind = "invalid_secret_pool"
	CannotClearNonSecret ErrorKind = "cannot_clear_non_secret"
)

// Error carries the original diagnostic without collapsing distinct API outcomes.
type Error struct {
	Kind    ErrorKind
	Message string
}

func (failure Error) Error() string { return failure.Message }

func settingError(kind ErrorKind, message string) error { return Error{kind, message} }
