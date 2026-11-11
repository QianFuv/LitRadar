package types

import "errors"

// Blob preserves NULL and empty blobs while rejecting SQLite text coercion.
type Blob []byte

// Scan accepts NULL or a native SQLite BLOB and copies its bytes.
func (value *Blob) Scan(source any) error {
	if source == nil {
		*value = nil
		return nil
	}
	bytes, ok := source.([]byte)
	if !ok {
		return errors.New("invalid SQLite blob column")
	}
	*value = append(Blob{}, bytes...)
	return nil
}
