//go:build windows && !server

package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"

	"github.com/QianFuv/LitRadar/apps/client/internal/nativehost"
	"github.com/danieljoos/wincred"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// newPlatformOptions returns the Windows facilities of the bridge: Credential
// Manager for tokens, the portable data directory for the host config, and the
// system save dialog and browser.
func newPlatformOptions() (nativehost.Options, error) {
	hostname, _ := os.Hostname()
	return nativehost.Options{
		Secrets:         credentialStore{},
		ConfigDirectory: portableDataDirectory,
		DeviceName:      "Windows " + hostname,
		SaveFile:        saveWithDialog,
		OpenURL:         openInBrowser,
		OidcCallbacks:   nativehost.LoopbackOidcCallbacks{},
		FocusApp:        focusMainWindow,
	}, nil
}

// openInBrowser opens rawURL in the system browser. Smoke builds can hand the
// URL to the smoke driver instead; see smokeOpenURL.
func openInBrowser(rawURL string) error {
	if isHandled, err := smokeOpenURL(rawURL); isHandled {
		return err
	}
	return application.Get().Browser.OpenURL(rawURL)
}

// focusMainWindow restores the application window and asks Windows to bring
// it to the foreground. Windows may only flash the taskbar button instead when
// another application owns the foreground.
func focusMainWindow() {
	for _, window := range application.Get().Window.GetAll() {
		window.Focus()
		return
	}
}

// portableDataDirectory returns the data folder next to the executable, so the
// portable client keeps its host config and WebView2 profile with its files.
func portableDataDirectory() (string, error) {
	executable, err := os.Executable()
	if err != nil {
		return "", err
	}
	return filepath.Join(filepath.Dir(executable), "data"), nil
}

// windowsOptions returns the WebView2 options of the desktop window: the
// profile lives in the portable data directory, and only smoke builds add
// browser arguments; see smokeBrowserArguments.
func windowsOptions() (application.WindowsOptions, error) {
	directory, err := portableDataDirectory()
	if err != nil {
		return application.WindowsOptions{}, err
	}
	return application.WindowsOptions{
		WebviewUserDataPath:   filepath.Join(directory, "webview2"),
		AdditionalBrowserArgs: smokeBrowserArguments(),
	}, nil
}

// credentialStore keeps secrets as generic credentials in Windows Credential Manager.
type credentialStore struct{}

// Get implements [nativehost.SecretStore].
func (credentialStore) Get(key string) (string, bool, error) {
	credential, err := wincred.GetGenericCredential(key)
	if errors.Is(err, wincred.ErrElementNotFound) {
		return "", false, nil
	}
	if err != nil {
		return "", false, err
	}
	return string(credential.CredentialBlob), true, nil
}

// Set implements [nativehost.SecretStore].
func (credentialStore) Set(key, value string) error {
	credential := wincred.NewGenericCredential(key)
	credential.UserName = "LitRadar"
	credential.CredentialBlob = []byte(value)
	credential.Persist = wincred.PersistLocalMachine
	return credential.Write()
}

// Delete implements [nativehost.SecretStore].
func (credentialStore) Delete(key string) error {
	credential, err := wincred.GetGenericCredential(key)
	if errors.Is(err, wincred.ErrElementNotFound) {
		return nil
	}
	if err != nil {
		return err
	}
	return credential.Delete()
}

// dialogCancelledMessage is the error text Wails returns when the user closes
// the Windows file dialog; the error value lives in an internal Wails package.
const dialogCancelledMessage = "cancelled by user"

// saveWithDialog asks for a destination with the system save dialog, starting
// in the Downloads folder, and writes data there.
func saveWithDialog(_ context.Context, filename string, data []byte) (bool, error) {
	app := application.Get()
	dialog := app.Dialog.SaveFile().SetFilename(filename).CanCreateDirectories(true)
	if home, err := os.UserHomeDir(); err == nil {
		downloads := filepath.Join(home, "Downloads")
		if info, err := os.Stat(downloads); err == nil && info.IsDir() {
			dialog.SetDirectory(downloads)
		}
	}
	if window := app.Window.Current(); window != nil {
		dialog.AttachToWindow(window)
	}
	destination, err := dialog.PromptForSingleSelection()
	if err != nil {
		if strings.Contains(err.Error(), dialogCancelledMessage) {
			return false, nil
		}
		return false, err
	}
	if destination == "" || destination == "." {
		return false, nil
	}
	if err := os.WriteFile(destination, data, 0o644); err != nil {
		return false, err
	}
	return true, nil
}
