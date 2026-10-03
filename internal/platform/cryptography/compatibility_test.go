package cryptography

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"testing"
)

func TestFrozenRustCryptoVectors(t *testing.T) {
	input, err := os.ReadFile("../../../tests/data/migration/rust/crypto.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Current struct {
			Password, Phc string
			Result        Verification
		}
		Legacy         struct{ Password, Salt, Hash string }
		Envelopes      []struct{ Context, Plaintext, Envelope string }
		PasswordPolicy []struct {
			Input    string
			Accepted bool
		}
	}
	if err := json.Unmarshal(input, &fixture); err != nil {
		t.Fatal(err)
	}
	if actual := VerifyPassword(fixture.Current.Password, "", fixture.Current.Phc); actual != fixture.Current.Result {
		t.Fatalf("Rust PHC mismatch: %s", actual)
	}
	if actual := LegacyHash(fixture.Legacy.Password, fixture.Legacy.Salt); actual != fixture.Legacy.Hash {
		t.Fatal("Rust legacy hash mismatch")
	}
	if VerifyPassword(fixture.Legacy.Password, fixture.Legacy.Salt, fixture.Legacy.Hash) != ValidLegacy {
		t.Fatal("Legacy classification lost")
	}
	for _, policy := range fixture.PasswordPolicy {
		if ValidNewPassword(policy.Input) != policy.Accepted {
			t.Fatalf("Unicode password length mismatch: %q", policy.Input)
		}
	}
	for _, replacement := range []string{"m=19457,t=2,p=1", "m=19456,t=3,p=1", "m=19456,t=2,p=2", "m=4294967295,t=2,p=1", "m=19456,t=2,p=1,keyid=YQ", "m=19456,m=19456,t=2,p=1"} {
		if VerifyPassword(fixture.Current.Password, "", strings.Replace(fixture.Current.Phc, "m=19456,t=2,p=1", replacement, 1)) != Invalid {
			t.Fatal("Accepted foreign PHC parameters")
		}
	}
	codec, err := NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer codec.Close()
	wrong, err := NewCodec(bytes.Repeat([]byte{43}, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer wrong.Close()
	for _, vector := range fixture.Envelopes {
		actual, err := codec.Decrypt(vector.Envelope, vector.Context)
		if err != nil || actual != vector.Plaintext {
			t.Fatalf("Rust envelope mismatch: %v", err)
		}
		if _, err := codec.Decrypt(vector.Envelope, vector.Context+"wrong"); !errors.Is(err, ErrAuthentication) {
			t.Fatal("Wrong AAD accepted")
		}
		if _, err := wrong.Decrypt(vector.Envelope, vector.Context); !errors.Is(err, ErrAuthentication) {
			t.Fatal("Wrong key accepted")
		}
		first, err := codec.Encrypt(vector.Plaintext, vector.Context)
		if err != nil {
			t.Fatal(err)
		}
		second, err := codec.Encrypt(vector.Plaintext, vector.Context)
		if err != nil || first == second {
			t.Fatal("Encryption reused nonce")
		}
		if recovered, err := codec.Decrypt(first, vector.Context); err != nil || recovered != vector.Plaintext {
			t.Fatal("Cipher round trip failed")
		}
	}
	if _, err := codec.Decrypt("plaintext", "context"); !errors.Is(err, ErrLegacyPlaintext) {
		t.Fatal("Plaintext accepted")
	}
	if cleared, err := codec.Decrypt("", "context"); err != nil || cleared != "" {
		t.Fatal("Cleared secret changed")
	}
	for _, object := range []any{codec, *codec} {
		if strings.Contains(fmt.Sprintf("%+v %#v", object, object), "42") {
			t.Fatal("Key leaked through formatting")
		}
	}
	codec.Close()
	if _, err := codec.Encrypt("secret", "context"); !errors.Is(err, ErrAuthentication) {
		t.Fatal("Closed codec reused zero key")
	}
}
