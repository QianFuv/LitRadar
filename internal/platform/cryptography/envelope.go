package cryptography

import (
	"crypto/rand"
	"encoding/base64"
	"errors"
	"strings"
	"unicode/utf8"

	"golang.org/x/crypto/chacha20poly1305"
)

const envelopePrefix = "litradarenc:v1:"

var (
	// ErrAuthentication intentionally excludes ciphertext and credential values.
	ErrAuthentication = errors.New("secret authentication failed")
	// ErrLegacyPlaintext means a normal runtime read encountered an unmigrated secret.
	ErrLegacyPlaintext = errors.New("legacy plaintext secret requires migration")
)

// Codec holds one owned key buffer; Close clears that buffer, not all compiler/runtime copies.
type Codec struct {
	key      [32]byte
	isClosed bool
}

// NewCodec requires an exact deployment key and copies it into the owned buffer.
func NewCodec(key []byte) (*Codec, error) {
	if len(key) != 32 {
		return nil, errors.New("deployment key must contain exactly 32 bytes")
	}
	codec := &Codec{}
	copy(codec.key[:], key)
	return codec, nil
}

// String prevents key disclosure through ordinary formatting.
func (codec Codec) String() string { return "SecretCodec([REDACTED])" }

// GoString prevents key disclosure through Go-syntax formatting.
func (codec Codec) GoString() string { return codec.String() }

// Close clears the directly owned key; callers must stop concurrent operations first.
func (codec *Codec) Close() { clear(codec.key[:]); codec.isClosed = true }

// Encrypt preserves the versioned nonce/ciphertext encoding and row-field associated data.
func (codec *Codec) Encrypt(plaintext, context string) (string, error) {
	if codec.isClosed {
		return "", ErrAuthentication
	}
	if plaintext == "" {
		return "", nil
	}
	cipher, err := chacha20poly1305.NewX(codec.key[:])
	if err != nil {
		return "", ErrAuthentication
	}
	nonce := make([]byte, chacha20poly1305.NonceSizeX)
	if _, err := rand.Read(nonce); err != nil {
		return "", errors.New("secret entropy unavailable")
	}
	ciphertext := cipher.Seal(nil, nonce, []byte(plaintext), []byte(context))
	return envelopePrefix + base64.RawURLEncoding.EncodeToString(nonce) + ":" + base64.RawURLEncoding.EncodeToString(ciphertext), nil
}

// Decrypt rejects invalid encodings, plaintext storage, wrong keys and wrong associated data.
func (codec *Codec) Decrypt(stored, context string) (string, error) {
	if codec.isClosed {
		return "", ErrAuthentication
	}
	if stored == "" {
		return "", nil
	}
	if !strings.HasPrefix(stored, envelopePrefix) {
		return "", ErrLegacyPlaintext
	}
	nonceText, cipherText, ok := strings.Cut(strings.TrimPrefix(stored, envelopePrefix), ":")
	if !ok {
		return "", ErrAuthentication
	}
	nonce, err := base64.RawURLEncoding.Strict().DecodeString(nonceText)
	if err != nil || len(nonce) != chacha20poly1305.NonceSizeX {
		return "", ErrAuthentication
	}
	ciphertext, err := base64.RawURLEncoding.Strict().DecodeString(cipherText)
	if err != nil {
		return "", ErrAuthentication
	}
	cipher, err := chacha20poly1305.NewX(codec.key[:])
	if err != nil {
		return "", ErrAuthentication
	}
	plaintext, err := cipher.Open(nil, nonce, ciphertext, []byte(context))
	if err != nil {
		return "", ErrAuthentication
	}
	defer clear(plaintext)
	if !utf8.Valid(plaintext) {
		return "", ErrAuthentication
	}
	return string(plaintext), nil
}
