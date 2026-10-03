package zjlib

import "testing"

func TestStateRoundTripRetainsExpiryAndCredentials(t *testing.T) {
	expires := int64(100)
	cookie := NewCookie("vpn358_sid", "secret", "proxy.test")
	cookie.Expires = &expires
	restoredCookie := CookieFromJson(cookie.Json())
	if restoredCookie.Expires == nil || *restoredCookie.Expires != expires {
		t.Error("cookie expiry lost in direct JSON value round trip")
	}
	client := NewClient(NewFixtureTransport(Success))
	client.LoadStateData(map[string]any{"bff_user_token": "token", "qr_uuid": "qr", "final_zyproxy_url": "https://proxy.test/kns55/", "fulltext_warmed_at": int64(100), "cookies": []any{map[string]any{"name": "vpn358_sid", "value": "secret", "expires": int64(100)}}})
	restored := NewClient(NewFixtureTransport(Success))
	restored.LoadStateData(client.StateData())
	state := restored.StateData()
	for key, expected := range map[string]string{"bff_user_token": "token", "qr_uuid": "qr", "final_zyproxy_url": "https://proxy.test/kns55/"} {
		if state[key] != expected {
			t.Errorf("%s did not survive direct state round trip", key)
		}
	}
	now := int64(101)
	if restored.HasFreshFulltextSession(&now) {
		t.Error("expired cookie became unexpired after state restoration")
	}
	exported := cookie.Json()
	exported["expires"] = int64(999)
	if *cookie.Expires != 100 {
		t.Error("exported cookie aliases source")
	}
}
