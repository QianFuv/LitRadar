//go:build !windows && !android && !server

package main

import (
	"errors"

	"github.com/QianFuv/LitRadar/apps/client/internal/nativehost"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// windowsOptions returns no Windows options outside Windows.
func windowsOptions() application.WindowsOptions {
	return application.WindowsOptions{}
}

// newPlatformOptions reports that only the Windows and Android native hosts exist.
func newPlatformOptions() (nativehost.Options, error) {
	return nativehost.Options{}, errors.New("litradar-client: native builds target only windows and android; use -tags server for the web host")
}
