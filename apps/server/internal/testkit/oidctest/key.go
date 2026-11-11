// Package oidctest signs real ID tokens and publishes verification keys for local provider fixtures.
package oidctest

import (
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"encoding/base64"
	"encoding/json"
	"testing"

	"github.com/go-jose/go-jose/v4"
)

// Key holds one signing key and its public JWKS projection.
type Key struct {
	signer jose.Signer
	public jose.JSONWebKey
}

// NewKey creates an independent RS256 key for one test provider.
func NewKey(t testing.TB) *Key {
	t.Helper()
	private, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	public := jose.JSONWebKey{Key: &private.PublicKey, Algorithm: string(jose.RS256), Use: "sig"}
	thumbprint, err := public.Thumbprint(crypto.SHA256)
	if err != nil {
		t.Fatal(err)
	}
	public.KeyID = base64.RawURLEncoding.EncodeToString(thumbprint)
	signer, err := jose.NewSigner(jose.SigningKey{Algorithm: jose.RS256, Key: jose.JSONWebKey{Key: private, KeyID: public.KeyID}}, (&jose.SignerOptions{}).WithType("JWT"))
	if err != nil {
		t.Fatal(err)
	}
	return &Key{signer: signer, public: public}
}

// JWKS returns the public key set served by the fixture.
func (key *Key) JWKS() jose.JSONWebKeySet {
	return jose.JSONWebKeySet{Keys: []jose.JSONWebKey{key.public}}
}

// Sign encodes and signs provider-controlled claims without bypassing token verification.
func (key *Key) Sign(claims any) (string, error) {
	payload, err := json.Marshal(claims)
	if err != nil {
		return "", err
	}
	signed, err := key.signer.Sign(payload)
	if err != nil {
		return "", err
	}
	return signed.CompactSerialize()
}
