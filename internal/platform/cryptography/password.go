// Package cryptography adapts maintained cryptographic primitives to persisted LitRadar formats.
package cryptography

import (
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"

	"golang.org/x/crypto/argon2"
	"golang.org/x/crypto/pbkdf2"
)

// Verification distinguishes current and legacy credentials without silently accepting other parameters.
type Verification string

const (
	Invalid      Verification = "Invalid"
	ValidCurrent Verification = "ValidCurrent"
	ValidLegacy  Verification = "ValidLegacy"
)

// ValidNewPassword counts Unicode scalar values rather than UTF-8 bytes.
func ValidNewPassword(password string) bool {
	return utf8.ValidString(password) && utf8.RuneCountInString(password) >= 12
}

// LegacyHash preserves the original PBKDF2-HMAC-SHA256 lowercase representation.
func LegacyHash(password, salt string) string {
	derived := pbkdf2.Key([]byte(password), []byte(salt), 260000, 32, sha256.New)
	defer clear(derived)
	return hex.EncodeToString(derived)
}

// VerifyPassword rejects unsupported PHC costs before beginning expensive work.
func VerifyPassword(password, legacySalt, stored string) Verification {
	if !strings.HasPrefix(stored, "$argon2") {
		actual := LegacyHash(password, legacySalt)
		if subtle.ConstantTimeCompare([]byte(actual), []byte(stored)) == 1 {
			return ValidLegacy
		}
		return Invalid
	}
	parts := strings.Split(stored, "$")
	if len(parts) != 6 || parts[0] != "" || parts[1] != "argon2id" || parts[2] != "v=19" {
		return Invalid
	}
	parameters := make(map[string]uint64)
	for _, field := range strings.Split(parts[3], ",") {
		key, value, ok := strings.Cut(field, "=")
		if !ok {
			return Invalid
		}
		if _, exists := parameters[key]; exists {
			return Invalid
		}
		parsed, err := strconv.ParseUint(value, 10, 32)
		if err != nil {
			return Invalid
		}
		parameters[key] = parsed
	}
	if len(parameters) != 3 || parameters["m"] != 19456 || parameters["t"] != 2 || parameters["p"] != 1 {
		return Invalid
	}
	salt, err := base64.RawStdEncoding.Strict().DecodeString(parts[4])
	if err != nil || len(salt) < 8 || len(parts[4]) > 64 {
		return Invalid
	}
	expected, err := base64.RawStdEncoding.Strict().DecodeString(parts[5])
	if err != nil || len(expected) != 32 {
		return Invalid
	}
	actual := argon2.IDKey([]byte(password), salt, 2, 19456, 1, 32)
	defer clear(actual)
	if subtle.ConstantTimeCompare(actual, expected) != 1 {
		return Invalid
	}
	return ValidCurrent
}

// HashPassword creates the fixed Argon2id PHC format with an independent random salt.
func HashPassword(password string) (string, error) {
	salt := make([]byte, 16)
	if _, err := rand.Read(salt); err != nil {
		return "", fmt.Errorf("password entropy unavailable")
	}
	derived := argon2.IDKey([]byte(password), salt, 2, 19456, 1, 32)
	defer clear(derived)
	return "$argon2id$v=19$m=19456,t=2,p=1$" + base64.RawStdEncoding.EncodeToString(salt) + "$" + base64.RawStdEncoding.EncodeToString(derived), nil
}
