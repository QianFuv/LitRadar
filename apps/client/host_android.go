//go:build android && !server

package main

import (
	"errors"
	"os/exec"
	"strings"

	"github.com/QianFuv/LitRadar/apps/client/internal/nativehost"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// appLinkOidcCallbacks receives the litradar://oidc/callback links that
// MainActivity forwards through deliverOidcCallback.
var appLinkOidcCallbacks = nativehost.NewAppLinkOidcCallbacks()

func init() {
	application.RegisterAndroidMain(main)
}

// newPlatformOptions returns the Android facilities of the bridge:
// EncryptedSharedPreferences for tokens, the app files directory for the host
// config, and app links for single sign-on callbacks. The Java host saves
// downloads through MediaStore, so the bridge has no save dialog here. The
// activity returns to the foreground by itself when the browser opens the
// callback link, so the bridge needs no focus hook.
func newPlatformOptions() (nativehost.Options, error) {
	return nativehost.Options{
		Secrets: androidSecretStore{},
		ConfigDirectory: func() (string, error) {
			directory := application.Android.StoragePath()
			if directory == "" {
				return "", errors.New("android storage path is unavailable")
			}
			return directory, nil
		},
		DeviceName: "Android " + androidModel(),
		OpenURL: func(rawURL string) error {
			application.Android.OpenURL(rawURL)
			return nil
		},
		OidcCallbacks: appLinkOidcCallbacks,
	}, nil
}

// windowsOptions returns no Windows options on Android.
func windowsOptions() (application.WindowsOptions, error) {
	return application.WindowsOptions{}, nil
}

// androidSecretStore keeps secrets in the Wails EncryptedSharedPreferences store.
type androidSecretStore struct{}

// Get implements [nativehost.SecretStore].
func (androidSecretStore) Get(key string) (string, bool, error) {
	return application.Android.SecureGet(key)
}

// Set implements [nativehost.SecretStore].
func (androidSecretStore) Set(key, value string) error {
	return application.Android.SecureSet(key, value)
}

// Delete implements [nativehost.SecretStore].
func (androidSecretStore) Delete(key string) error {
	return application.Android.SecureDelete(key)
}

// androidModel returns the device model name, or "device" when it is unknown.
func androidModel() string {
	output, err := exec.Command("/system/bin/getprop", "ro.product.model").Output()
	model := strings.TrimSpace(string(output))
	if err != nil || model == "" {
		return "device"
	}
	return model
}
