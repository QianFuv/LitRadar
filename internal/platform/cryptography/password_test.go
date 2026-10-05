package cryptography

import (
	"strings"
	"testing"
)

func TestCurrentPasswordsUseIndependentSaltsAndRejectWrongCredentials(t *testing.T) {
	password := "IndependentPassword!2026"
	first, err := HashPassword(password)
	if err != nil {
		t.Fatal(err)
	}
	second, err := HashPassword(password)
	if err != nil {
		t.Fatal(err)
	}
	if first == second || strings.Split(first, "$")[4] == strings.Split(second, "$")[4] {
		t.Fatal("password hashes reused salt")
	}
	for _, stored := range []string{first, second} {
		if !strings.HasPrefix(stored, "$argon2id$v=19$m=19456,t=2,p=1$") || VerifyPassword(password, "", stored) != ValidCurrent {
			t.Fatal("current password format or verification changed")
		}
		if VerifyPassword("wrong password", "", stored) != Invalid {
			t.Fatal("wrong password accepted")
		}
	}
	for _, parameters := range []string{"m=19457,t=2,p=1", "m=19456,t=3,p=1", "m=19456,t=2,p=2", "m=4294967295,t=2,p=1", "m=19456,t=2,p=1,keyid=1", "m=19456,m=19456,t=2,p=1"} {
		if VerifyPassword(password, "", strings.Replace(first, "m=19456,t=2,p=1", parameters, 1)) != Invalid {
			t.Fatal("unsupported password parameters accepted")
		}
	}
}

func TestPasswordPolicyCountsUnicodeScalars(t *testing.T) {
	for _, unit := range []string{"a", "界", "😀"} {
		if ValidNewPassword(strings.Repeat(unit, 11)) || !ValidNewPassword(strings.Repeat(unit, 12)) {
			t.Fatalf("password length counted incorrectly for %q", unit)
		}
	}
	if ValidNewPassword(strings.Repeat("a", 12) + "\xff") {
		t.Fatal("invalid UTF-8 accepted")
	}
}
