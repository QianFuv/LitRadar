//go:build litradar_web

// Package webassets provides the immutable frontend compiled into production builds.
package webassets

import (
	"embed"
	"io/fs"
)

//go:embed all:export
var assets embed.FS

// Files returns the complete production export, including its CSP manifest.
func Files() (fs.FS, error) { return fs.Sub(assets, "export") }
