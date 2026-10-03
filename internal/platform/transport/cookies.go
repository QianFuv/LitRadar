package transport

import "net/http/cookiejar"

// NewCookieJar creates an independently owned jar for one user's upstream session.
// Public-suffix filtering is explicitly disabled to match reqwest's existing default
// cookie_store policy; ordinary domain, path, expiry and Secure rules still apply.
func NewCookieJar() *cookiejar.Jar {
	jar, err := cookiejar.New(&cookiejar.Options{PublicSuffixList: nil})
	if err != nil {
		panic(err)
	}
	return jar
}
