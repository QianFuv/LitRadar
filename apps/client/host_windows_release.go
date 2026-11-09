//go:build windows && !server && !smoke

package main

// smokeBrowserArguments adds no WebView2 browser arguments to release builds.
func smokeBrowserArguments() []string {
	return nil
}

// smokeOpenURL leaves every URL to the system browser in release builds.
func smokeOpenURL(string) (bool, error) {
	return false, nil
}
