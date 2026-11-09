// Package cfp embeds the reviewed source registry and immutable original-language seed.
package cfp

import _ "embed"

const SeedId = "reviewed-2026-09-15-v1"

//go:embed seed.json
var seed []byte

//go:embed sources.json
var registry []byte

// Seed returns an independent copy of the exact reviewed import bytes.
func Seed() []byte { return append([]byte(nil), seed...) }

// Registry returns the packaged source registrations without exposing mutable shared storage.
func Registry() []byte { return append([]byte(nil), registry...) }
