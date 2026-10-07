package api

import (
	"container/list"
	"net/netip"
	"strings"
	"sync"
	"time"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type authAttempt uint8

const (
	loginAttempt authAttempt = iota
	registerAttempt
)

type authClientSource struct {
	address netip.Addr
	class   string
}
type rateLimitRejection struct {
	retryAfter                  uint64
	reason, bucket, sourceClass string
	rejectedCount               uint64
	shouldPersistAudit          bool
}
type bucketKey struct {
	kind     authAttempt
	address  netip.Addr
	username string
}
type rejectionKey struct {
	kind   authAttempt
	bucket string
}
type tokenBucket struct{ available, lastRefill uint64 }
type trackedBucket struct {
	bucket   tokenBucket
	lastUsed uint64
	key      bucketKey
	element  *list.Element
}
type authRateLimiter struct {
	mutex                       sync.Mutex
	policy                      settings.RateLimitPolicy
	started                     time.Time
	ipBuckets, usernameBuckets  map[bucketKey]*trackedBucket
	ipOrder, usernameOrder      list.List
	globalLogin, globalRegister tokenBucket
	nextSequence                uint64
	rejectionCounts, lastAudit  map[rejectionKey]uint64
}

func newAuthRateLimiter(policy settings.RateLimitPolicy) *authRateLimiter {
	return &authRateLimiter{
		policy: policy, started: time.Now(), ipBuckets: make(map[bucketKey]*trackedBucket), usernameBuckets: make(map[bucketKey]*trackedBucket),
		globalLogin: fullBucket(policy.GlobalLogin, 0), globalRegister: fullBucket(policy.GlobalRegister, 0),
		rejectionCounts: make(map[rejectionKey]uint64), lastAudit: make(map[rejectionKey]uint64),
	}
}

func (limiter *authRateLimiter) check(kind authAttempt, source authClientSource, username string) *rateLimitRejection {
	return limiter.checkAt(kind, source, username, uint64(time.Since(limiter.started)/time.Second))
}

func (limiter *authRateLimiter) checkAt(kind authAttempt, source authClientSource, username string, now uint64) *rateLimitRejection {
	limiter.mutex.Lock()
	defer limiter.mutex.Unlock()
	limiter.nextSequence = saturatingIncrement(limiter.nextSequence)
	ipPolicy := limiter.policy.LoginIp
	if kind == registerAttempt {
		ipPolicy = limiter.policy.RegisterIp
	}
	retry := acquireTracked(limiter.ipBuckets, &limiter.ipOrder, bucketKey{kind: kind, address: source.address}, limiter.policy.IpKeyLimit, ipPolicy, now, limiter.nextSequence)
	if retry != 0 {
		return limiter.rejection(kind, "client_ip", source.class, retry, now)
	}
	retry = acquireTracked(limiter.usernameBuckets, &limiter.usernameOrder, bucketKey{kind: kind, username: normalizeUsername(username)}, limiter.policy.UsernameKeyLimit, limiter.policy.Username, now, limiter.nextSequence)
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

func (limiter *authRateLimiter) clearUsername(kind authAttempt, username string) {
	limiter.mutex.Lock()
	defer limiter.mutex.Unlock()
	key := bucketKey{kind: kind, username: normalizeUsername(username)}
	if tracked := limiter.usernameBuckets[key]; tracked != nil {
		limiter.usernameOrder.Remove(tracked.element)
		delete(limiter.usernameBuckets, key)
	}
}

func (limiter *authRateLimiter) rejection(kind authAttempt, bucket, source string, retry, now uint64) *rateLimitRejection {
	key := rejectionKey{kind, bucket}
	count := saturatingIncrement(limiter.rejectionCounts[key])
	limiter.rejectionCounts[key] = count
	lastAudit, hasAudit := limiter.lastAudit[key]
	shouldAudit := !hasAudit || (now >= lastAudit && now-lastAudit >= 60)
	if shouldAudit {
		limiter.lastAudit[key] = now
	}
	return &rateLimitRejection{retry, "rate_limit_exceeded", bucket, source, count, shouldAudit}
}

func fullBucket(policy settings.TokenBucketPolicy, now uint64) tokenBucket {
	return tokenBucket{policy.Capacity * policy.RefillSeconds, now}
}

func (bucket *tokenBucket) acquire(now uint64, policy settings.TokenBucketPolicy) uint64 {
	if now > bucket.lastRefill {
		maximum := policy.Capacity * policy.RefillSeconds
		missing := maximum - bucket.available
		elapsed := now - bucket.lastRefill
		if elapsed >= (missing+policy.RefillTokens-1)/policy.RefillTokens {
			bucket.available = maximum
		} else {
			bucket.available += elapsed * policy.RefillTokens
		}
		bucket.lastRefill = now
	}
	if bucket.available >= policy.RefillSeconds {
		bucket.available -= policy.RefillSeconds
		return 0
	}
	return (policy.RefillSeconds - bucket.available + policy.RefillTokens - 1) / policy.RefillTokens
}

// acquireTracked reuses an evicted node before recording this ordered token acquisition.
func acquireTracked(buckets map[bucketKey]*trackedBucket, order *list.List, key bucketKey, limit uint64, policy settings.TokenBucketPolicy, now, sequence uint64) uint64 {
	tracked := buckets[key]
	if tracked == nil {
		if uint64(len(buckets)) >= limit && len(buckets) > 0 {
			tracked = trackedEviction(buckets, order, sequence)
			delete(buckets, tracked.key)
		} else {
			tracked = &trackedBucket{}
			tracked.element = order.PushBack(tracked)
		}
		tracked.key, tracked.bucket = key, fullBucket(policy, now)
		buckets[key] = tracked
	}
	tracked.lastUsed = sequence
	order.MoveToBack(tracked.element)
	return tracked.bucket.acquire(now, policy)
}

// trackedEviction retains deterministic key ordering when the usage sequence is saturated.
func trackedEviction(buckets map[bucketKey]*trackedBucket, order *list.List, sequence uint64) *trackedBucket {
	tracked := order.Front().Value.(*trackedBucket)
	if sequence != ^uint64(0) {
		return tracked
	}
	for candidate, entry := range buckets {
		if entry.lastUsed < tracked.lastUsed || (entry.lastUsed == tracked.lastUsed && bucketKeyLess(candidate, tracked.key)) {
			tracked = entry
		}
	}
	return tracked
}

func bucketKeyLess(first, second bucketKey) bool {
	if first.kind != second.kind {
		return first.kind < second.kind
	}
	if first.address.IsValid() {
		return first.address.Compare(second.address) < 0
	}
	return first.username < second.username
}

func saturatingIncrement(value uint64) uint64 {
	if value != ^uint64(0) {
		return value + 1
	}
	return value
}

func normalizeUsername(value string) string {
	characters := []rune(strings.TrimSpace(value))
	if len(characters) > 32 {
		characters = characters[:32]
	}
	return asciiLower(string(characters))
}
