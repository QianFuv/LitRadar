//go:build windows && !server && !smoke

package main

// smokeBrowserArguments adds no WebView2 browser arguments to release builds.
func smokeBrowserArguments() []string {
	return nil
}
