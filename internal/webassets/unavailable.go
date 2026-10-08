//go:build !litradar_web

// Package webassets keeps backend development independent of frontend compilation.
package webassets

import (
	"errors"
	"io/fs"
)

// Files rejects production startup when the executable has no compiled frontend.
func Files() (fs.FS, error) {
	return nil, errors.New("production frontend is unavailable; stage the web export and build with -tags litradar_web")
}
