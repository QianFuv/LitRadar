package types

import (
	"errors"
	"unicode/utf8"
)

// SecretText validates encrypted storage without coercion or invalid UTF-8.
type SecretText string

// Scan accepts only native SQLite text containing valid UTF-8.
func (value *SecretText) Scan(source any) error {
	decoded, ok := source.(string)
	if !ok || !utf8.ValidString(decoded) {
		return errors.New("invalid secret storage text column")
	}
	*value = SecretText(decoded)
	return nil
}
