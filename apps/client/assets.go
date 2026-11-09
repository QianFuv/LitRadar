// Command litradar-client is the LitRadar client host built with Wails v3.
//
// With the server build tag it serves the web client as a Wails server build:
// it only serves the embedded Vite build, binds no services, emits no events,
// and hides the Wails runtime namespace; API traffic reaches the Go server
// through the external reverse proxy, not through this process.
//
// Without the server tag it runs the Windows desktop or Android app. The
// nativehost Bridge service performs every server request with the bearer
// token kept in OS secure storage, so the frontend never sees the token.
package main

import "embed"

//go:embed all:frontend/dist
var embeddedAssets embed.FS
