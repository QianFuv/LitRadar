package types

// Boolean returns the SQLite integer representation of a boolean.
func Boolean(value bool) int64 {
	if value {
		return 1
	}
	return 0
}
