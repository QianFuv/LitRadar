//go:build windows && !server && smoke

package main

import (
	"os"
	"strconv"
)

// smokeBrowserURLFileEnv names a file that receives the URLs a smoke build
// would open in the system browser, so the smoke driver can open them in a
// browser it controls. Release builds do not contain this file.
const smokeBrowserURLFileEnv = "LITRADAR_CLIENT_SMOKE_BROWSER_URL_FILE"

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

// smokeOpenURL appends rawURL to the file named by smokeBrowserURLFileEnv and
// reports whether it did, instead of opening the system browser.
func smokeOpenURL(rawURL string) (bool, error) {
	path := os.Getenv(smokeBrowserURLFileEnv)
	if path == "" {
		return false, nil
	}
	file, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return true, err
	}
	_, writeErr := file.WriteString(rawURL + "\n")
	closeErr := file.Close()
	if writeErr != nil {
		return true, writeErr
	}
	return true, closeErr
}
