package api

import (
	"fmt"
	"net/netip"
	"strings"
	"sync"
	"testing"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func rateLimitTestPolicy() settings.RateLimitPolicy {
	return settings.RateLimitPolicy{
		LoginIp:        settings.TokenBucketPolicy{Capacity: 20, RefillTokens: 1, RefillSeconds: 10},
		RegisterIp:     settings.TokenBucketPolicy{Capacity: 20, RefillTokens: 1, RefillSeconds: 10},
		Username:       settings.TokenBucketPolicy{Capacity: 2, RefillTokens: 1, RefillSeconds: 10},
		GlobalLogin:    settings.TokenBucketPolicy{Capacity: 20, RefillTokens: 1, RefillSeconds: 10},
		GlobalRegister: settings.TokenBucketPolicy{Capacity: 20, RefillTokens: 1, RefillSeconds: 10},
		IpKeyLimit:     2, UsernameKeyLimit: 2,
	}
}

func directSource(address string) authClientSource {
	return authClientSource{netip.MustParseAddr(address), "direct"}
}

func TestRateLimitUsernameNormalizationAndRetryDelay(t *testing.T) {
	limiter := newAuthRateLimiter(rateLimitTestPolicy())
	source := directSource("192.0.2.10")
	if limiter.checkAt(loginAttempt, source, " Alice ", 100) != nil || limiter.checkAt(loginAttempt, source, "alice", 101) != nil {
		t.Fatal("initial attempts rejected")
	}
	failure := limiter.checkAt(loginAttempt, source, "ALICE", 102)
	if failure == nil || failure.retryAfter != 8 || failure.bucket != "username" || failure.reason != "rate_limit_exceeded" || failure.sourceClass != "direct" || failure.rejectedCount != 1 || !failure.shouldPersistAudit {
		t.Fatalf("rejection: %+v", failure)
	}
	limiter.clearUsername(registerAttempt, "ALIce")
	if failure := limiter.checkAt(loginAttempt, source, "alice", 106); failure == nil || failure.bucket != "username" {
		t.Fatal("registration cleared login bucket")
	}
	limiter.clearUsername(loginAttempt, "ALIce")
	if limiter.checkAt(loginAttempt, source, "alice", 106) != nil {
		t.Fatal("successful login did not clear username")
	}
	if normalizeUsername(" İALICE ") != "İalice" || utf8.RuneCountInString(normalizeUsername(strings.Repeat("密", 1000))) != 32 {
		t.Fatal("Unicode normalization changed")
	}
}

func TestRateLimitAuditsAreBoundedAndCountsContinue(t *testing.T) {
	policy := rateLimitTestPolicy()
	policy.Username = settings.TokenBucketPolicy{Capacity: 1, RefillTokens: 1, RefillSeconds: 3600}
	limiter := newAuthRateLimiter(policy)
	source := directSource("192.0.2.20")
	if limiter.checkAt(loginAttempt, source, "sampled", 100) != nil {
		t.Fatal("first attempt")
	}
	for index, now := range []uint64{101, 159, 161} {
		failure := limiter.checkAt(loginAttempt, source, "sampled", now)
		if failure == nil || failure.rejectedCount != uint64(index+1) || failure.shouldPersistAudit != (index != 1) {
			t.Fatalf("at %d: %+v", now, failure)
		}
	}
}

func TestRateLimitRejectsBeforeTouchingLaterBuckets(t *testing.T) {
	policy := rateLimitTestPolicy()
	policy.Username = settings.TokenBucketPolicy{Capacity: 1, RefillTokens: 1, RefillSeconds: 60}
	policy.GlobalLogin = settings.TokenBucketPolicy{Capacity: 2, RefillTokens: 1, RefillSeconds: 60}
	limiter := newAuthRateLimiter(policy)
	if limiter.checkAt(loginAttempt, directSource("192.0.2.1"), "alpha", 10) != nil {
		t.Fatal("first attempt")
	}
	if failure := limiter.checkAt(loginAttempt, directSource("192.0.2.1"), "alpha", 10); failure == nil || failure.bucket != "username" {
		t.Fatal("username not limited")
	}
	if limiter.checkAt(loginAttempt, directSource("192.0.2.2"), "beta", 10) != nil {
		t.Fatal("username rejection consumed global capacity")
	}
	if failure := limiter.checkAt(loginAttempt, directSource("192.0.2.3"), "gamma", 10); failure == nil || failure.bucket != "global_breaker" {
		t.Fatal("global not limited")
	}
	policy.LoginIp = settings.TokenBucketPolicy{Capacity: 1, RefillTokens: 1, RefillSeconds: 60}
	limiter = newAuthRateLimiter(policy)
	if limiter.checkAt(loginAttempt, directSource("192.0.2.1"), "alpha", 20) != nil {
		t.Fatal("first attempt")
	}
	if failure := limiter.checkAt(loginAttempt, directSource("192.0.2.1"), "beta", 20); failure == nil || failure.bucket != "client_ip" {
		t.Fatal("IP not limited")
	}
	if _, exists := limiter.usernameBuckets[bucketKey{kind: loginAttempt, username: "beta"}]; exists {
		t.Fatal("IP rejection created username state")
	}
}

func TestRateLimitLruBoundsBothOperationsAndSaturationIsStable(t *testing.T) {
	limiter := newAuthRateLimiter(rateLimitTestPolicy())
	for _, scenario := range []struct{ address, username string }{{"192.0.2.1", "alpha"}, {"192.0.2.2", "beta"}, {"192.0.2.1", "alpha"}, {"192.0.2.3", "gamma"}} {
		if limiter.checkAt(loginAttempt, directSource(scenario.address), scenario.username, 10) != nil {
			t.Fatal("fixture rejected")
		}
	}
	if limiter.usernameBuckets[bucketKey{kind: loginAttempt, username: "beta"}] != nil {
		t.Fatal("recent use did not protect alpha")
	}
	for index := range 100 {
		limiter.checkAt(registerAttempt, directSource(fmt.Sprintf("203.0.113.%d", index)), strings.Repeat("密", 1000), 20)
	}
	if len(limiter.ipBuckets) > 2 || len(limiter.usernameBuckets) > 2 {
		t.Fatal("unbounded keyed state")
	}
	for key := range limiter.usernameBuckets {
		if utf8.RuneCountInString(key.username) > 32 {
			t.Fatal("unbounded username")
		}
	}
	policy := settings.TokenBucketPolicy{Capacity: 1, RefillTokens: 1, RefillSeconds: 10}
	buckets := make(map[bucketKey]*trackedBucket)
	for _, name := range []string{"z", "a", "m"} {
		acquireTracked(buckets, bucketKey{username: name}, 2, policy, 0, ^uint64(0))
	}
	if buckets[bucketKey{username: "a"}] != nil || buckets[bucketKey{username: "z"}] == nil {
		t.Fatal("saturated sequence must evict by key order")
	}
	bucket := fullBucket(policy, 0)
	if bucket.acquire(0, policy) != 0 || bucket.acquire(9, policy) != 1 || bucket.acquire(^uint64(0), policy) != 0 {
		t.Fatal("integer refill/overflow changed")
	}
}

func TestRateLimiterConcurrentRequestsShareCapacity(t *testing.T) {
	limiter := newAuthRateLimiter(rateLimitTestPolicy())
	var workers sync.WaitGroup
	for range 100 {
		workers.Go(func() { limiter.checkAt(loginAttempt, directSource("192.0.2.1"), "same", 0) })
	}
	workers.Wait()
	if limiter.rejectionCounts[rejectionKey{loginAttempt, "username"}] != 18 || limiter.rejectionCounts[rejectionKey{loginAttempt, "client_ip"}] != 80 {
		t.Fatal("concurrent bucket accounting changed")
	}
}
