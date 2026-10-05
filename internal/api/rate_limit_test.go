package api

import (
	"container/list"
	"fmt"
	"math/rand/v2"
	"net/netip"
	"reflect"
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
	var order list.List
	for _, name := range []string{"z", "a", "m"} {
		acquireTracked(buckets, &order, bucketKey{username: name}, 2, policy, 0, ^uint64(0))
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

func referenceRateLimitCheck(limiter *authRateLimiter, kind authAttempt, source authClientSource, username string, now uint64) *rateLimitRejection {
	limiter.mutex.Lock()
	defer limiter.mutex.Unlock()
	limiter.nextSequence = saturatingIncrement(limiter.nextSequence)
	ipPolicy := limiter.policy.LoginIp
	if kind == registerAttempt {
		ipPolicy = limiter.policy.RegisterIp
	}
	retry := referenceAcquireTracked(limiter.ipBuckets, bucketKey{kind: kind, address: source.address}, limiter.policy.IpKeyLimit, ipPolicy, now, limiter.nextSequence)
	if retry != 0 {
		return limiter.rejection(kind, "client_ip", source.class, retry, now)
	}
	retry = referenceAcquireTracked(limiter.usernameBuckets, bucketKey{kind: kind, username: normalizeUsername(username)}, limiter.policy.UsernameKeyLimit, limiter.policy.Username, now, limiter.nextSequence)
	if retry != 0 {
		return limiter.rejection(kind, "username", source.class, retry, now)
	}
	if kind == loginAttempt {
		retry = limiter.globalLogin.acquire(now, limiter.policy.GlobalLogin)
	} else {
		retry = limiter.globalRegister.acquire(now, limiter.policy.GlobalRegister)
	}
	if retry != 0 {
		return limiter.rejection(kind, "global_breaker", source.class, retry, now)
	}
	return nil
}

func referenceAcquireTracked(buckets map[bucketKey]*trackedBucket, key bucketKey, limit uint64, policy settings.TokenBucketPolicy, now, sequence uint64) uint64 {
	tracked := buckets[key]
	if tracked == nil {
		if uint64(len(buckets)) >= limit {
			var oldestKey bucketKey
			var oldest *trackedBucket
			for candidate, entry := range buckets {
				if oldest == nil || entry.lastUsed < oldest.lastUsed || (entry.lastUsed == oldest.lastUsed && bucketKeyLess(candidate, oldestKey)) {
					oldestKey, oldest = candidate, entry
				}
			}
			delete(buckets, oldestKey)
		}
		tracked = &trackedBucket{bucket: fullBucket(policy, now)}
		buckets[key] = tracked
	}
	tracked.lastUsed = sequence
	return tracked.bucket.acquire(now, policy)
}

func TestRateLimitRandomizedReferenceTrace(t *testing.T) {
	for _, limit := range []uint64{1, 2, 8} {
		for _, sequence := range []uint64{0, ^uint64(0) - 3} {
			t.Run(fmt.Sprintf("limit=%d/sequence=%d", limit, sequence), func(t *testing.T) {
				policy := rateLimitTestPolicy()
				policy.IpKeyLimit, policy.UsernameKeyLimit = limit, limit
				actual, reference := newAuthRateLimiter(policy), newAuthRateLimiter(policy)
				actual.nextSequence, reference.nextSequence = sequence, sequence
				random := rand.New(rand.NewPCG(17, 43))
				names := []string{" Alice ", "ALICE", "beta", "Gamma", "İALICE", strings.Repeat("密", 40), "", "z"}
				for step := range 3000 {
					kind := authAttempt(random.IntN(2))
					username := names[random.IntN(len(names))]
					if random.IntN(7) == 0 {
						actual.clearUsername(kind, username)
						delete(reference.usernameBuckets, bucketKey{kind: kind, username: normalizeUsername(username)})
					} else {
						address := fmt.Sprintf("192.0.2.%d", random.IntN(16))
						if random.IntN(2) == 0 {
							address = fmt.Sprintf("2001:db8::%x", random.IntN(16))
						}
						source := directSource(address)
						now := uint64(random.IntN(150))
						got, want := actual.checkAt(kind, source, username, now), referenceRateLimitCheck(reference, kind, source, username, now)
						if !reflect.DeepEqual(got, want) {
							t.Fatalf("step %d rejection got %+v want %+v", step, got, want)
						}
					}
					for index, order := range []*list.List{&actual.ipOrder, &actual.usernameOrder} {
						buckets := actual.ipBuckets
						if index == 1 {
							buckets = actual.usernameBuckets
						}
						if order.Len() != len(buckets) {
							t.Fatalf("step %d list/map length differs", step)
						}
						seen := map[bucketKey]bool{}
						for element := order.Front(); element != nil; element = element.Next() {
							tracked := element.Value.(*trackedBucket)
							if seen[tracked.key] || buckets[tracked.key] != tracked || tracked.element != element {
								t.Fatalf("step %d stale list entry", step)
							}
							seen[tracked.key] = true
						}
					}
					for index, pair := range [][2]map[bucketKey]*trackedBucket{{actual.ipBuckets, reference.ipBuckets}, {actual.usernameBuckets, reference.usernameBuckets}} {
						if len(pair[0]) != len(pair[1]) || uint64(len(pair[0])) > limit {
							t.Fatalf("step %d map %d length", step, index)
						}
						for key, want := range pair[1] {
							got := pair[0][key]
							if got == nil || got.bucket != want.bucket || got.lastUsed != want.lastUsed {
								t.Fatalf("step %d map %d key %+v differs", step, index, key)
							}
						}
					}
					if actual.globalLogin != reference.globalLogin || actual.globalRegister != reference.globalRegister || actual.nextSequence != reference.nextSequence || !reflect.DeepEqual(actual.rejectionCounts, reference.rejectionCounts) || !reflect.DeepEqual(actual.lastAudit, reference.lastAudit) {
						t.Fatalf("step %d accounting differs", step)
					}
				}
			})
		}
	}
}
