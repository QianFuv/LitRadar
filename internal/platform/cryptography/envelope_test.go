package cryptography

import (
	"bytes"
	"encoding/base64"
	"errors"
	"strings"
	"testing"
)

func TestSecretEnvelopeAuthenticatesFreshDataAndOwnsItsKey(t *testing.T) {
	key := bytes.Repeat([]byte{17}, 32)
	codec, err := NewCodec(key)
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	clear(key)
	reader, err := NewCodec(bytes.Repeat([]byte{17}, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	wrong, err := NewCodec(bytes.Repeat([]byte{18}, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer wrong.Close()
	plaintext, context := "独立 secret 😀", "row:7:password"
	first, err := codec.Encrypt(plaintext, context)
	if err != nil {
		t.Fatal(err)
	}
	second, err := codec.Encrypt(plaintext, context)
	if err != nil || first == second {
		t.Fatalf("nonce reused: %v", err)
	}
	for _, stored := range []string{first, second} {
		if actual, err := reader.Decrypt(stored, context); err != nil || actual != plaintext {
			t.Fatalf("independent reader failed: %v", err)
		}
		if _, err := reader.Decrypt(stored, "row:8:password"); !errors.Is(err, ErrAuthentication) {
			t.Fatal("wrong associated data accepted")
		}
		if _, err := wrong.Decrypt(stored, context); !errors.Is(err, ErrAuthentication) {
			t.Fatal("wrong key accepted")
		}
	}
	parts := strings.Split(first, ":")
	ciphertext, err := base64.RawURLEncoding.DecodeString(parts[3])
	if err != nil {
		t.Fatal(err)
	}
	ciphertext[0] ^= 1
	parts[3] = base64.RawURLEncoding.EncodeToString(ciphertext)
	if _, err := reader.Decrypt(strings.Join(parts, ":"), context); !errors.Is(err, ErrAuthentication) {
		t.Fatal("tampered ciphertext accepted")
	}
	for _, invalid := range []string{envelopePrefix, envelopePrefix + "bad:bad", envelopePrefix + ":"} {
		if _, err := reader.Decrypt(invalid, context); !errors.Is(err, ErrAuthentication) {
			t.Fatal("malformed envelope accepted")
		}
	}
	if _, err := reader.Decrypt("plaintext", context); !errors.Is(err, ErrLegacyPlaintext) {
		t.Fatal("plaintext silently accepted")
	}
	if stored, err := codec.Encrypt("", context); err != nil || stored != "" {
		t.Fatal("empty secret changed")
	}
	if actual, err := reader.Decrypt("", context); err != nil || actual != "" {
		t.Fatal("cleared secret changed")
	}
	codec.Close()
	if _, err := codec.Encrypt(plaintext, context); !errors.Is(err, ErrAuthentication) {
		t.Fatal("closed codec encrypted data")
	}
	if _, err := codec.Decrypt(first, context); !errors.Is(err, ErrAuthentication) {
		t.Fatal("closed codec decrypted data")
	}
	for _, size := range []int{0, 31, 33} {
		if _, err := NewCodec(make([]byte, size)); err == nil {
			t.Fatalf("accepted key length %d", size)
		}
	}
}
