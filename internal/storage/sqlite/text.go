package sqlite

import (
	"errors"
	"unicode/utf8"
)

// Text refuses SQLite coercions and invalid UTF-8 that the original typed string reader rejected.
type Text string

// Scan accepts only a native SQLite TEXT value containing valid UTF-8.
func (value *Text) Scan(source any) error {
	decoded, ok := source.(string)
	if !ok || !utf8.ValidString(decoded) {
		return errors.New("invalid SQLite text column")
	}
	*value = Text(decoded)
	return nil
}

// OptionalText preserves NULL while applying the same strict text storage-class boundary.
type OptionalText struct{ Value *string }

// Scan accepts NULL or valid native SQLite TEXT without coercion.
func (value *OptionalText) Scan(source any) error {
	if source == nil {
		value.Value = nil
		return nil
	}
	var decoded Text
	if err := decoded.Scan(source); err != nil {
		return err
	}
	text := string(decoded)
	value.Value = &text
	return nil
}

// Integer refuses coercion from REAL, TEXT or BLOB in typed SQLite reads.
type Integer int64

// Scan accepts only a native SQLite INTEGER value.
func (value *Integer) Scan(source any) error {
	decoded, ok := source.(int64)
	if !ok {
		return errors.New("invalid SQLite integer column")
	}
	*value = Integer(decoded)
	return nil
}

// OptionalInteger preserves NULL without weakening integer storage-class validation.
type OptionalInteger struct{ Value *int64 }

// Scan accepts NULL or a native SQLite INTEGER without numeric coercion.
func (value *OptionalInteger) Scan(source any) error {
	if source == nil {
		value.Value = nil
		return nil
	}
	var decoded Integer
	if err := decoded.Scan(source); err != nil {
		return err
	}
	integer := int64(decoded)
	value.Value = &integer
	return nil
}

// Number accepts only the INTEGER and REAL storage classes supported by the original floating reader.
type Number float64

// Scan preserves floating timestamps without accepting numeric text or blobs.
func (value *Number) Scan(source any) error {
	switch decoded := source.(type) {
	case float64:
		*value = Number(decoded)
	case int64:
		*value = Number(decoded)
	default:
		return errors.New("invalid SQLite numeric column")
	}
	return nil
}
