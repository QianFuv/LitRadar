//go:build windows && !server && smoke

package main

import (
	"os"
	"strconv"
)

// smokeRemoteDebuggingPortEnv names the WebView2 remote debugging port of a smoke
// build. Wails clears WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS at startup, so the
// smoke test opens the CDP endpoint through this variable instead; release
// builds do not contain this file.
const smokeRemoteDebuggingPortEnv = "LITRADAR_CLIENT_REMOTE_DEBUGGING_PORT"

// smokeBrowserArguments opens the loopback CDP endpoint the smoke test drives.
func smokeBrowserArguments() []string {
	port, err := strconv.Atoi(os.Getenv(smokeRemoteDebuggingPortEnv))
	if err != nil || port <= 0 || port > 65535 {
		return nil
	}
	return []string{"--remote-debugging-port=" + strconv.Itoa(port)}
}
