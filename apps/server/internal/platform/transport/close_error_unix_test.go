//go:build !windows

package transport

func isPlatformPeerCloseError(error) bool { return false }
