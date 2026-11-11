//go:build !server

package main

import (
	"io/fs"
	"log/slog"
	"net/http"
	"os"

	"github.com/QianFuv/LitRadar/apps/client/internal/nativehost"
	"github.com/QianFuv/LitRadar/apps/client/internal/webhost"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// main reports startup failures before exiting with a nonzero status.
func main() {
	if err := run(); err != nil {
		slog.Error("client.failed", "error", err)
		os.Exit(1)
	}
}

func run() error {
	dist, err := fs.Sub(embeddedAssets, "frontend/dist")
	if err != nil {
		return err
	}
	handler, err := webhost.NewHandler(dist)
	if err != nil {
		return err
	}
	options, err := newPlatformOptions()
	if err != nil {
		return err
	}
	windows, err := windowsOptions()
	if err != nil {
		return err
	}
	app := application.New(application.Options{
		Name:        "LitRadar",
		Description: "LitRadar literature search, favorites, and tracking client",
		Services:    []application.Service{application.NewService(nativehost.NewBridge(options))},
		Windows:     windows,
		Assets: application.AssetOptions{
			Handler:        handler,
			Middleware:     securityHeaders,
			DisableLogging: true,
		},
	})
	app.Window.NewWithOptions(application.WebviewWindowOptions{
		Title:     "LitRadar",
		Width:     1280,
		Height:    820,
		MinWidth:  360,
		MinHeight: 560,
		URL:       "/",
	})
	return app.Run()
}

// securityHeaders applies the web host hardening headers to the embedded
// assets. Unlike the server build it leaves the Wails runtime namespace open,
// because the frontend reaches the Bridge service through it.
func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		webhost.SetSecurityHeaders(writer.Header(), 0)
		next.ServeHTTP(writer, request)
	})
}
