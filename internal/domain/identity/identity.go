// Package identity preserves persisted integer identities and their public decimal JSON encoding.
package identity

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"strconv"
)

// Id is an exact signed SQLite integer serialized as a decimal JSON string.
type Id int64

// MarshalJSON retains precision for JavaScript clients.
func (value Id) MarshalJSON() ([]byte, error) {
	return json.Marshal(strconv.FormatInt(int64(value), 10))
}

// UnmarshalJSON accepts decimal strings and integral JSON numbers without float conversion.
func (value *Id) UnmarshalJSON(input []byte) error {
	var text string
	if len(input) > 0 && input[0] == '"' {
		if err := json.Unmarshal(input, &text); err != nil {
			return err
		}
	} else {
		decoder := json.NewDecoder(bytes.NewReader(input))
		decoder.UseNumber()
		var decoded any
		if err := decoder.Decode(&decoded); err != nil {
			return err
		}
		number, ok := decoded.(json.Number)
		if !ok {
			return fmt.Errorf("expected a decimal identifier")
		}
		text = string(number)
	}
	parsed, err := strconv.ParseInt(text, 10, 64)
	if err != nil {
		return fmt.Errorf("invalid SQLite identifier: %w", err)
	}
	*value = Id(parsed)
	return nil
}

// Stable returns an existing integer or the baseline SHA-256-derived positive identifier.
func Stable(value, prefix string) Id {
	if parsed, err := strconv.ParseInt(value, 10, 64); err == nil {
		return Id(parsed)
	}
	digest := sha256.Sum256([]byte(prefix + ":" + value))
	result := binary.BigEndian.Uint64(digest[:8]) & 0x7fffffffffffffff
	if result == 0 {
		return 1
	}
	return Id(result)
}
