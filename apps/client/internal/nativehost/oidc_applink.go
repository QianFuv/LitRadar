package nativehost

import (
	"context"
	"crypto/subtle"
	"net/url"
	"sync"
)

// AppLinkOidcCallbacks receives sign-in callbacks that the operating system
// delivers to the app as [AppLinkOidcRedirectURI] links, as the Android host
// does. At most one sign-in waits at a time; a new one supersedes the old.
type AppLinkOidcCallbacks struct {
	mutex         sync.Mutex
	pending       *appLinkListener
	hasListened   bool
	isInterrupted bool
}

// NewAppLinkOidcCallbacks returns a callback source without a pending sign-in.
func NewAppLinkOidcCallbacks() *AppLinkOidcCallbacks {
	return &AppLinkOidcCallbacks{}
}

// Listen implements [OidcCallbackSource].
func (callbacks *AppLinkOidcCallbacks) Listen(state string) (OidcCallbackListener, error) {
	listener := &appLinkListener{
		owner:      callbacks,
		state:      state,
		callbacks:  make(chan url.Values, 1),
		superseded: make(chan struct{}),
	}
	callbacks.mutex.Lock()
	defer callbacks.mutex.Unlock()
	if callbacks.pending != nil {
		close(callbacks.pending.superseded)
	}
	callbacks.pending = listener
	callbacks.hasListened = true
	callbacks.isInterrupted = false
	return listener, nil
}

// Restart forgets every sign-in of the previous app page. The Android host
// calls it when the activity, and with it the page that started any pending
// sign-in, is created anew; the pending sign-in ends as superseded and a
// callback for it then reports an interruption.
func (callbacks *AppLinkOidcCallbacks) Restart() {
	callbacks.mutex.Lock()
	defer callbacks.mutex.Unlock()
	if callbacks.pending != nil {
		close(callbacks.pending.superseded)
		callbacks.pending = nil
	}
	callbacks.hasListened = false
}

// ConsumeInterruption implements [OidcCallbackSource].
func (callbacks *AppLinkOidcCallbacks) ConsumeInterruption() bool {
	callbacks.mutex.Lock()
	defer callbacks.mutex.Unlock()
	isInterrupted := callbacks.isInterrupted
	callbacks.isInterrupted = false
	return isInterrupted
}

// Deliver hands a link the app received to the waiting sign-in and reports
// whether it was accepted. Links other than [AppLinkOidcRedirectURI] and links
// with another state are ignored. A callback that arrives before the current
// page started any sign-in marks an interruption: the sign-in it belonged to
// was lost with an earlier process or activity. A callback for a sign-in the
// current page cancelled is ignored.
func (callbacks *AppLinkOidcCallbacks) Deliver(rawURI string) bool {
	link, err := url.Parse(rawURI)
	if err != nil || link.Scheme+"://"+link.Host+link.Path != AppLinkOidcRedirectURI || link.User != nil || link.Fragment != "" {
		return false
	}
	query := link.Query()
	states := query["state"]
	callbacks.mutex.Lock()
	defer callbacks.mutex.Unlock()
	listener := callbacks.pending
	if listener == nil {
		if !callbacks.hasListened {
			callbacks.isInterrupted = true
		}
		return false
	}
	if len(states) != 1 || subtle.ConstantTimeCompare([]byte(states[0]), []byte(listener.state)) != 1 {
		return false
	}
	callbacks.pending = nil
	listener.callbacks <- query
	return true
}

// appLinkListener is one sign-in waiting for its app link.
type appLinkListener struct {
	owner      *AppLinkOidcCallbacks
	state      string
	callbacks  chan url.Values
	superseded chan struct{}
}

// RedirectURI implements [OidcCallbackListener].
func (listener *appLinkListener) RedirectURI() string {
	return AppLinkOidcRedirectURI
}

// Wait implements [OidcCallbackListener].
func (listener *appLinkListener) Wait(ctx context.Context) (url.Values, error) {
	select {
	case query := <-listener.callbacks:
		return query, nil
	case <-listener.superseded:
		return nil, errOidcSuperseded
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// Close implements [OidcCallbackListener].
func (listener *appLinkListener) Close() {
	listener.owner.mutex.Lock()
	defer listener.owner.mutex.Unlock()
	if listener.owner.pending == listener {
		listener.owner.pending = nil
	}
}
