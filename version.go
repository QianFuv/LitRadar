// Package litradar exposes the version shared by binaries and release automation.
package litradar

import (
	_ "embed"
	"strings"
)

//go:embed VERSION
var version string

// Version returns the release version embedded at build time.
func Version() string { return strings.TrimSpace(version) }
