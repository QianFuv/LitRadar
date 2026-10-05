package api

import (
	"fmt"
	"net/netip"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func BenchmarkRateLimit(b *testing.B) {
	for _, limits := range []struct{ ip, username int }{{64, 64}, {8192, 4096}} {
		b.Run(fmt.Sprintf("Churn%d_%d", limits.ip, limits.username), func(b *testing.B) {
			policy := settings.TokenBucketPolicy{Capacity: 1, RefillTokens: 1, RefillSeconds: 1}
			limiter := newAuthRateLimiter(settings.RateLimitPolicy{LoginIp: policy, RegisterIp: policy, Username: policy, GlobalLogin: policy, GlobalRegister: policy, IpKeyLimit: uint64(limits.ip), UsernameKeyLimit: uint64(limits.username)})
			count := max(limits.ip, limits.username) + 1
			sources := make([]authClientSource, count)
			usernames := make([]string, count)
			for index := range count {
				sources[index] = authClientSource{address: netip.AddrFrom4([4]byte{192, 0, byte(index >> 8), byte(index)}), class: "direct"}
				usernames[index] = fmt.Sprintf("synthetic-user-%d", index)
			}
			var now uint64
			for index := range count - 1 {
				now++
				if rejection := limiter.checkAt(loginAttempt, sources[index], usernames[index], now); rejection != nil {
					b.Fatal("fixture unexpectedly rejected", rejection)
				}
			}
			index := count - 1
			b.ReportAllocs()
			for b.Loop() {
				now++
				if rejection := limiter.checkAt(loginAttempt, sources[index], usernames[index], now); rejection != nil {
					b.Fatal("churn unexpectedly rejected", rejection)
				}
				index = (index + 1) % count
			}
		})
	}
}
