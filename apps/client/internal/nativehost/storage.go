package nativehost

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
)

// SecretStore is the OS secure storage used for bearer tokens: Windows
// Credential Manager on desktop and EncryptedSharedPreferences on Android.
type SecretStore interface {
	// Get returns the value stored under key and whether it exists.
	Get(key string) (string, bool, error)
	// Set stores value under key, replacing any previous value.
	Set(key, value string) error
	// Delete removes key; deleting a missing key is not an error.
	Delete(key string) error
}

// tokenKey is the secure storage key of the bearer token for one server origin.
func tokenKey(origin string) string {
	return "LitRadar/token/" + origin
}

// configFileName is the host configuration file inside the config directory.
const configFileName = "client.json"

// hostConfig is the non-secret host configuration persisted as JSON.
type hostConfig struct {
	ServerOrigin string `json:"server_origin"`
}

// ConfigStore persists the selected server origin in a JSON file. The origin is
// not secret, so it lives in the user config directory rather than secure storage.
type ConfigStore struct {
	path string
}

// NewConfigStore returns a store that keeps its file in directory.
func NewConfigStore(directory string) *ConfigStore {
	return &ConfigStore{path: filepath.Join(directory, configFileName)}
}

// Load returns the stored server origin, or "" when none is stored. A stored
// value that is no longer a valid origin is treated as absent.
func (store *ConfigStore) Load() (string, error) {
	content, err := os.ReadFile(store.path)
	if errors.Is(err, fs.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", newBridgeError(CodeStorageFailure, "read host config: "+err.Error())
	}
	var config hostConfig
	if json.Unmarshal(content, &config) != nil {
		return "", nil
	}
	origin, err := NormalizeServerOrigin(config.ServerOrigin)
	if err != nil {
		return "", nil
	}
	return origin, nil
}

// Save stores origin, replacing the file atomically.
func (store *ConfigStore) Save(origin string) error {
	content, err := json.Marshal(hostConfig{ServerOrigin: origin})
	if err != nil {
		return newBridgeError(CodeStorageFailure, "encode host config: "+err.Error())
	}
	if err := os.MkdirAll(filepath.Dir(store.path), 0o700); err != nil {
		return newBridgeError(CodeStorageFailure, "create config directory: "+err.Error())
	}
	temporary, err := os.CreateTemp(filepath.Dir(store.path), configFileName+".*.tmp")
	if err != nil {
		return newBridgeError(CodeStorageFailure, "create host config: "+err.Error())
	}
	temporaryPath := temporary.Name()
	_, writeErr := temporary.Write(content)
	closeErr := temporary.Close()
	if writeErr == nil {
		writeErr = closeErr
	}
	if writeErr == nil {
		writeErr = os.Rename(temporaryPath, store.path)
	}
	if writeErr != nil {
		_ = os.Remove(temporaryPath)
		return newBridgeError(CodeStorageFailure, "write host config: "+writeErr.Error())
	}
	return nil
}

// Clear removes the stored origin.
func (store *ConfigStore) Clear() error {
	if err := os.Remove(store.path); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return newBridgeError(CodeStorageFailure, "remove host config: "+err.Error())
	}
	return nil
}
