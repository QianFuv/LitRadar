//go:build server

package main

import (
	"io/fs"
	"log"
	"os"

	"github.com/QianFuv/LitRadar/apps/client/internal/webhost"
	"github.com/wailsapp/wails/v3/pkg/application"
)

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	config, err := webhost.LoadConfig(os.LookupEnv)
	if err != nil {
		return err
	}
	dist, err := fs.Sub(embeddedAssets, "frontend/dist")
	if err != nil {
		return err
	}
	handler, err := webhost.NewHandler(dist)
	if err != nil {
		return err
	}
	app := application.New(application.Options{
		Name:        "LitRadar",
		Description: "LitRadar web client",
		Assets: application.AssetOptions{
			Handler:        handler,
			Middleware:     webhost.SecurityMiddleware(config),
			DisableLogging: true,
		},
		Server: application.ServerOptions{
			Host:                    config.Host,
			Port:                    config.Port,
			WebSocketOriginPatterns: config.OriginPatterns,
		},
	})
	return app.Run()
}
