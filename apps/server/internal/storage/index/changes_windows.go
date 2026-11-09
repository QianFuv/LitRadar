package index

import "golang.org/x/sys/windows"

func syncManifestDirectory(path string) error { return nil }
func removeManifestFile(path string) error {
	value, err := windows.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	return windows.DeleteFile(value)
}
