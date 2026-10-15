package api

import (
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

// usernameNormalizationReference preserves the full conversion used before bounded prefix collection.
func usernameNormalizationReference(value string) string {
	characters := []rune(strings.TrimSpace(value))
	if len(characters) > 32 {
		characters = characters[:32]
	}
	return asciiLower(string(characters))
}

// FuzzUsernameNormalization retains exact rate-limit bucket identity for arbitrary strings.
func FuzzUsernameNormalization(f *testing.F) {
	for _, value := range []string{"", " Alice ", strings.Repeat("A", 32), strings.Repeat("A", 33), "\u2003İALICE密\u3000", strings.Repeat("密A", 17), "\xff\xfe\xc3A", strings.Repeat("a", 31) + "\xffz"} {
		f.Add(value)
	}
	f.Fuzz(func(t *testing.T, value string) {
		if len(value) > 8192 {
			t.Skip()
		}
		if actual, expected := normalizeUsername(value), usernameNormalizationReference(value); actual != expected {
			t.Fatalf("bucket identity changed: %q != %q for %q", actual, expected, value)
		}
	})
}

// TestRateLimitLongUsernamesShareBoundedBuckets preserves prefix collisions and successful-login clearing.
func TestRateLimitLongUsernamesShareBoundedBuckets(t *testing.T) {
	prefix := strings.Repeat("密A", 16)
	first := "\u2003" + prefix + strings.Repeat("X", 1<<20)
	second := prefix + strings.Repeat("Y", 1<<20)
	limiter := newAuthRateLimiter(rateLimitTestPolicy())
	source := directSource("192.0.2.45")
	if limiter.checkAt(loginAttempt, source, first, 100) != nil || limiter.checkAt(loginAttempt, source, second, 100) != nil {
		t.Fatal("initial prefix attempts were rejected")
	}
	if rejection := limiter.checkAt(loginAttempt, source, prefix, 100); rejection == nil || rejection.bucket != "username" {
		t.Fatal("long suffixes escaped the shared username limit", rejection)
	}
	limiter.clearUsername(loginAttempt, second)
	if limiter.checkAt(loginAttempt, source, first, 100) != nil {
		t.Fatal("long username did not clear the same bucket")
	}
}

// BenchmarkUsernameAdmission includes the shared limiter lock and advancing refill clock.
func BenchmarkUsernameAdmission(b *testing.B) {
	for _, scenario := range []struct {
		name  string
		value string
	}{
		{"normal", "Synthetic-User"},
		{"ascii-1KiB", strings.Repeat("A", 1024)},
		{"ascii-1MiB", strings.Repeat("A", 1<<20)},
		{"unicode-1MiB", strings.Repeat("密", (1<<20)/3)},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			policy := settings.TokenBucketPolicy{Capacity: 1, RefillTokens: 1, RefillSeconds: 1}
			limiter := newAuthRateLimiter(settings.RateLimitPolicy{LoginIp: policy, RegisterIp: policy, Username: policy, GlobalLogin: policy, GlobalRegister: policy, IpKeyLimit: 1, UsernameKeyLimit: 1})
			source := directSource("192.0.2.45")
			var now uint64
			b.ReportAllocs()
			for b.Loop() {
				now++
				if rejection := limiter.checkAt(loginAttempt, source, scenario.value, now); rejection != nil {
					b.Fatal("refilled admission changed", rejection)
				}
			}
		})
	}
}
