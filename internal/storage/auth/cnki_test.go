package auth

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

func testCnki(t *testing.T) (*Repository, *CnkiSessions) {
	t.Helper()
	repository := testRepository(t)
	testAdmin(t, repository)
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	sessions := NewCnkiSessions(repository, codec)
	sessions.now = func() float64 { return 100 }
	return repository, sessions
}

func jwt(expiration string) string {
	return "unsigned." + base64.RawURLEncoding.EncodeToString([]byte(`{"exp":`+expiration+`}`)) + ".unverified"
}

func sessionJson(t *testing.T, token string) json.RawMessage {
	t.Helper()
	encoded, err := json.Marshal(map[string]any{"bff_user_token": token, "cookies": []any{map[string]string{"name": " cookie ", "value": "synthetic-secret"}, map[string]string{"name": "cookie"}, map[string]string{"name": " "}}})
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}

// TestCnkiStatusHidesSecretsAndPreservesCookieOrder checks credential-free status and private state availability.
func TestCnkiStatusHidesSecretsAndPreservesCookieOrder(t *testing.T) {
	repository, sessions := testCnki(t)
	ctx := context.Background()
	token := jwt("200.75")
	summary, err := sessions.Upsert(ctx, 1, sessionJson(t, token), "active", nil)
	if err != nil {
		t.Fatal(err)
	}
	assertCnkiStatusProjection(t, summary, token)
	stored := queryScalar[string](t, repository, "SELECT session_json FROM cnki_sessions")
	if !strings.HasPrefix(stored, "litradarenc:v1:") || strings.Contains(stored, token) {
		t.Fatal("session persisted plaintext")
	}
	assertCnkiRawStateRedaction(t, sessions, ctx, token)
}

// TestCnkiReserveKeepsActiveSessionAndClearFencesLateCompletion checks pending reservation and clear generation fencing.
func TestCnkiReserveKeepsActiveSessionAndClearFencesLateCompletion(t *testing.T) {
	repository, sessions := testCnki(t)
	ctx := context.Background()
	qr := "old-qr"
	data := sessionJson(t, jwt("200"))
	if _, err := sessions.Upsert(ctx, 1, data, "active", &qr); err != nil {
		t.Fatal(err)
	}
	if touched, err := sessions.TouchUsed(ctx, 1); err != nil || !touched {
		t.Fatal(err)
	}
	before := queryScalar[string](t, repository, "SELECT session_json FROM cnki_sessions")
	sessions.now = func() float64 { return 110 }
	generation := assertCnkiReserveKeepsActive(t, repository, sessions, ctx, before)
	assertCnkiClearFencesLateCompletion(t, repository, sessions, ctx, generation, data)
	assertCnkiEmptyProjection(t, sessions, ctx)
}

func TestCnkiGenerationAndExactQrMustBothMatch(t *testing.T) {
	_, sessions := testCnki(t)
	ctx := context.Background()
	qr := "current-qr"
	wrong := "different-qr"
	if _, err := sessions.Upsert(ctx, 1, json.RawMessage(`{"qr_uuid":"fallback"}`), "waiting_scan", &qr); err != nil {
		t.Fatal(err)
	}
	if result, err := sessions.Complete(ctx, 1, 1, &wrong, sessionJson(t, jwt("200")), "active", nil); err != nil || result != nil {
		t.Fatal("wrong QR matched")
	}
	if result, err := sessions.Complete(ctx, 1, 1, &qr, sessionJson(t, jwt("200")), "active", nil); err != nil || result == nil {
		t.Fatal(err)
	}
	if result, err := sessions.Complete(ctx, 1, 1, &qr, sessionJson(t, jwt("200")), "active", nil); err != nil || result != nil {
		t.Fatal("old generation matched")
	}
}

func TestCnkiRawJsonStrictButSummaryTolerant(t *testing.T) {
	repository, sessions := testCnki(t)
	ctx := context.Background()
	if _, err := sessions.Upsert(ctx, 1, json.RawMessage(`{}`), "legacy_unknown", nil); err != nil {
		t.Fatal(err)
	}
	encrypted, err := sessions.codec.Encrypt("bad-json", secrets.CnkiContext(1))
	if err != nil {
		t.Fatal(err)
	}
	runSql(t, repository, "UPDATE cnki_sessions SET session_json=?", encrypted)
	if _, err := sessions.Data(ctx, 1, false); err == nil {
		t.Fatal("raw loader accepted corrupt JSON")
	}
	if summary, err := sessions.Status(ctx, 1); err != nil || summary.Status != "legacy_unknown" || summary.HasBffUserToken {
		t.Fatalf("%+v %v", summary, err)
	}
}

func TestCnkiExpiryBoundaryUnknownTokenAndPermissivePayloadDecoder(t *testing.T) {
	_, sessions := testCnki(t)
	ctx := context.Background()
	for _, item := range []struct {
		token, status string
		remaining     *int64
	}{
		{jwt("100"), "expired", pointerInt64(0)},
		{jwt("99.99"), "expired", pointerInt64(0)},
		{jwt("101.9"), "active", pointerInt64(1)},
		{"unparseable-token", "active", nil},
		{jwt(`"100"`), "active", nil},
	} {
		summary, err := sessions.Upsert(ctx, 1, sessionJson(t, item.token), "waiting_scan", nil)
		if err != nil || summary.Status != item.status || !reflect.DeepEqual(summary.SecondsRemaining, item.remaining) {
			t.Fatalf("%+v %v", summary, err)
		}
	}
	token := jwt("200")
	parts := strings.Split(token, ".")
	parts[1] = "=" + strings.Join(strings.Split(parts[1], ""), "=") + "="
	if expires := jwtExpiration(strings.Join(parts, ".")); expires == nil || *expires != 200 {
		t.Fatal("legacy embedded padding rejected")
	}
	if expires := jwtExpiration("header." + base64.RawURLEncoding.EncodeToString([]byte(`{"exp":200 }`)) + "A.signature"); expires == nil || *expires != 200 {
		t.Fatal("legacy incomplete trailing sextet rejected")
	}
}

func pointerInt64(value int64) *int64 { return &value }

func TestCnkiClearingAbsentRowStillCreatesFenceAndQrFallback(t *testing.T) {
	repository, sessions := testCnki(t)
	ctx := context.Background()
	if existed, err := sessions.Clear(ctx, 1); err != nil || existed {
		t.Fatalf("%t %v", existed, err)
	}
	if generation := queryScalar[int64](t, repository, "SELECT generation FROM cnki_sessions"); generation != 1 {
		t.Fatal(generation)
	}
	blank := "  "
	if _, err := sessions.Upsert(ctx, 1, json.RawMessage(`{"qr_uuid":" fallback "}`), "unknown", &blank); err != nil {
		t.Fatal(err)
	}
	if data, err := sessions.Data(ctx, 1, false); err != nil || data.QrUuid != "fallback" || data.Status != "unknown" {
		t.Fatalf("%v %v", data, err)
	}
	if summary, err := sessions.Status(ctx, 1); err != nil || summary.Status != "waiting_scan" {
		t.Fatalf("%+v %v", summary, err)
	}
}

// assertCnkiStatusProjection checks effective expiry, ordered cookie names and public secret omission.
func assertCnkiStatusProjection(t *testing.T, summary CnkiStatus, token string) {
	t.Helper()
	if !summary.Configured || summary.Status != "active" || !summary.HasBffUserToken || *summary.ExpiresAt != 200.75 || *summary.SecondsRemaining != 100 || !reflect.DeepEqual(summary.CookieNames, []string{"cookie", "cookie"}) {
		t.Fatalf("%+v", summary)
	}
	encoded, err := json.Marshal(summary)
	if err != nil || strings.Contains(string(encoded), token) || strings.Contains(string(encoded), "synthetic-secret") {
		t.Fatal("status leaked credentials")
	}
}

// assertCnkiRawStateRedaction checks private raw availability without formatting or JSON disclosure.
func assertCnkiRawStateRedaction(t *testing.T, sessions *CnkiSessions, ctx context.Context, token string) {
	t.Helper()
	data, err := sessions.Data(ctx, 1, false)
	if err != nil || data == nil || !strings.Contains(string(data.SessionData), token) {
		t.Fatalf("%v %v", data, err)
	}
	if strings.Contains(fmt.Sprintf("%+v %#v", data, data), token) {
		t.Fatal("raw state logged")
	}
	encoded, err := json.Marshal(data)
	if err != nil || strings.Contains(string(encoded), token) {
		t.Fatal("raw state serialized")
	}
}

// assertCnkiReserveKeepsActive checks generation advancement without replacing completed credentials.
func assertCnkiReserveKeepsActive(t *testing.T, repository *Repository, sessions *CnkiSessions, ctx context.Context, before string) int64 {
	t.Helper()
	generation, err := sessions.Reserve(ctx, 1)
	if err != nil || generation != 2 {
		t.Fatalf("%d %v", generation, err)
	}
	if stored := queryScalar[string](t, repository, "SELECT session_json FROM cnki_sessions"); stored != before {
		t.Fatal("reserve replaced active credentials")
	}
	if updated := queryScalar[float64](t, repository, "SELECT updated_at FROM cnki_sessions"); updated != 100 {
		t.Fatal("reserve touched last completion")
	}
	active, err := sessions.Data(ctx, 1, true)
	if err != nil || active == nil || active.QrUuid != "" || active.Generation != 2 {
		t.Fatalf("%v %v", active, err)
	}
	return generation
}

// assertCnkiClearFencesLateCompletion checks the clear tombstone rejects a captured late completion.
func assertCnkiClearFencesLateCompletion(t *testing.T, repository *Repository, sessions *CnkiSessions, ctx context.Context, generation int64, data json.RawMessage) {
	t.Helper()
	if existed, err := sessions.Clear(ctx, 1); err != nil || !existed {
		t.Fatalf("%t %v", existed, err)
	}
	if result, err := sessions.Complete(ctx, 1, generation, nil, data, "active", nil); err != nil || result != nil {
		t.Fatal("late completion resurrected cleared credentials")
	}
	if count := queryScalar[int](t, repository, "SELECT count(*) FROM cnki_sessions WHERE status='empty' AND generation=3 AND last_used_at IS NULL"); count != 1 {
		t.Fatal("missing generation tombstone")
	}
}

// assertCnkiEmptyProjection checks tombstones cannot be touched or exposed as visible session data.
func assertCnkiEmptyProjection(t *testing.T, sessions *CnkiSessions, ctx context.Context) {
	t.Helper()
	if touched, err := sessions.TouchUsed(ctx, 1); err != nil || touched {
		t.Fatalf("%t %v", touched, err)
	}
	if data, err := sessions.Data(ctx, 1, false); err != nil || data != nil {
		t.Fatal("tombstone visible")
	}
	if summary, err := sessions.Status(ctx, 1); err != nil || summary.Configured || summary.Status != "empty" || summary.CookieNames == nil {
		t.Fatalf("%+v %v", summary, err)
	}
}
