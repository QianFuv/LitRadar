package auth

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"log/slog"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
)

// CnkiSessions persists encrypted state while fencing network completions with monotonic generations.
type CnkiSessions struct {
	repository *Repository
	codec      *secrets.Codec
	now        func() float64
}

// NewCnkiSessions shares the auth pool and a caller-owned deployment codec.
func NewCnkiSessions(repository *Repository, codec *secrets.Codec) *CnkiSessions {
	return &CnkiSessions{repository, codec, func() float64 { return float64(time.Now().UnixNano()) / 1e9 }}
}

// CnkiData is private source-adapter state and never an ordinary API response.
type CnkiData struct {
	SessionData json.RawMessage `json:"-"`
	QrUUID      string          `json:"-"`
	Status      string          `json:"-"`
	Generation  int64           `json:"-"`
}

func (data CnkiData) String() string       { return "CnkiData([REDACTED])" }
func (data CnkiData) GoString() string     { return data.String() }
func (data CnkiData) LogValue() slog.Value { return slog.StringValue(data.String()) }

// CnkiStatus is a credential-free lifecycle projection with explicit nullable timestamps.
type CnkiStatus struct {
	Configured       bool     `json:"configured" nullable:"false"`
	Status           string   `json:"status" nullable:"false"`
	HasBffUserToken  bool     `json:"has_bff_user_token" nullable:"false"`
	ExpiresAt        *float64 `json:"expires_at" required:"false" nullable:"true" format:"double"`
	SecondsRemaining *int64   `json:"seconds_remaining" required:"false" nullable:"true" format:"int64"`
	CookieNames      []string `json:"cookie_names" nullable:"false"`
	UpdatedAt        *float64 `json:"updated_at" required:"false" nullable:"true" format:"double"`
	LastUsedAt       *float64 `json:"last_used_at" required:"false" nullable:"true" format:"double"`
}

type cnkiRow struct {
	plaintext, qrUUID, status string
	updated, lastUsed         *float64
	generation                int64
}

func (sessions *CnkiSessions) row(ctx context.Context, user identity.ID) (*cnkiRow, error) {
	var row cnkiRow
	err := func() error {
		queryResult, err := queries.CNKISession(ctx, sessions.repository.database, authdb.CNKISessionParams{UserID: int64(user)})
		if err != nil {
			return err
		}
		row.plaintext = queryResult.SessionJSON
		row.qrUUID = queryResult.QRUUID
		row.status = queryResult.Status
		row.updated = new(queryResult.UpdatedAt)
		row.lastUsed = queryResult.LastUsedAt
		row.generation = queryResult.Generation
		return nil
	}()
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	row.plaintext, err = sessions.codec.Decrypt(row.plaintext, secrets.CnkiContext(int64(user)))
	if err != nil {
		return nil, err
	}
	return &row, nil
}

func decodeSession(raw string) (any, error) {
	if !jsonvalue.ValidJSON(raw) {
		return nil, errors.New("invalid CNKI session JSON")
	}
	decoder := json.NewDecoder(strings.NewReader(raw))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, errors.New("invalid CNKI session JSON")
	}
	return value, nil
}

func sessionField(value any, field string) any {
	object, _ := value.(map[string]any)
	return object[field]
}
func sessionString(value any, field string) string {
	text, _ := sessionField(value, field).(string)
	return text
}

// Data strictly decodes persisted state, unlike the tolerant status projection.
func (sessions *CnkiSessions) Data(ctx context.Context, user identity.ID, isActiveOnly bool) (*CnkiData, error) {
	row, err := sessions.row(ctx, user)
	if err != nil || row == nil {
		return nil, err
	}
	value, err := decodeSession(row.plaintext)
	if err != nil {
		return nil, err
	}
	status := row.status
	if isActiveOnly {
		status = effectiveCnkiStatus(value, row, sessions.now())
		if status != "active" {
			return nil, nil
		}
	}
	return &CnkiData{json.RawMessage(row.plaintext), row.qrUUID, status, row.generation}, nil
}

// Status tolerates malformed decrypted JSON while retaining effective token expiry rules.
func (sessions *CnkiSessions) Status(ctx context.Context, user identity.ID) (CnkiStatus, error) {
	row, err := sessions.row(ctx, user)
	if err != nil {
		return CnkiStatus{}, err
	}
	return summarizeCnki(row, sessions.now()), nil
}

// Reserve clears only the pending QR identifier on conflict, preserving an existing active session.
func (sessions *CnkiSessions) Reserve(ctx context.Context, user identity.ID) (int64, error) {
	empty, err := sessions.codec.Encrypt("{}", secrets.CnkiContext(int64(user)))
	if err != nil {
		return 0, err
	}
	var generation int64
	err = func() error {
		queryResult, err := queries.ReserveCNKISession(ctx, sessions.repository.database, authdb.ReserveCNKISessionParams{UserID: int64(user), SessionJSON: empty, CreatedAt: sessions.now()})
		if err != nil {
			return err
		}
		generation = queryResult
		return nil
	}()
	return generation, err
}

func (sessions *CnkiSessions) prepare(user identity.ID, data json.RawMessage, status string, qrUUID *string, now float64) (cnkiRow, string, *float64, error) {
	value, err := decodeSession(string(data))
	if err != nil {
		return cnkiRow{}, "", nil, err
	}
	plaintext, err := jsonvalue.EncodeJSON(value)
	if err != nil {
		return cnkiRow{}, "", nil, err
	}
	resolved := ""
	if qrUUID != nil {
		resolved = strings.TrimSpace(*qrUUID)
	}
	if resolved == "" {
		resolved = strings.TrimSpace(sessionString(value, "qr_uuid"))
	}
	expires := jwtExpiration(sessionString(value, "bff_user_token"))
	encrypted, err := sessions.codec.Encrypt(plaintext, secrets.CnkiContext(int64(user)))
	return cnkiRow{plaintext: plaintext, qrUUID: resolved, status: status, updated: &now}, encrypted, expires, err
}

// Complete publishes only the matching generation and optional exact QR identity.
func (sessions *CnkiSessions) Complete(ctx context.Context, user identity.ID, generation int64, expectedQr *string, data json.RawMessage, status string, qrUUID *string) (*CnkiStatus, error) {
	now := sessions.now()
	row, encrypted, expires, err := sessions.prepare(user, data, status, qrUUID, now)
	if err != nil {
		return nil, err
	}
	result, err := queries.CompleteCNKISession(ctx, sessions.repository.database, authdb.CompleteCNKISessionParams{SessionJSON: encrypted, QRUUID: row.qrUUID, Status: row.status, TokenExpiresAt: expires, UpdatedAt: now, UserID: int64(user), Generation: generation, Column8: expectedQr})
	if err != nil {
		return nil, err
	}
	count, err := result.RowsAffected()
	if err != nil || count == 0 {
		return nil, err
	}
	row.generation = generation
	if generation < math.MaxInt64 {
		row.generation++
	}
	summary := summarizeCnki(&row, now)
	return &summary, nil
}

// Upsert atomically replaces session data while preserving the original creation and last-use timestamps.
func (sessions *CnkiSessions) Upsert(ctx context.Context, user identity.ID, data json.RawMessage, status string, qrUUID *string) (CnkiStatus, error) {
	now := sessions.now()
	row, encrypted, expires, err := sessions.prepare(user, data, status, qrUUID, now)
	if err != nil {
		return CnkiStatus{}, err
	}
	err = func() error {
		queryResult, err := queries.StoreCNKISession(ctx, sessions.repository.database, authdb.StoreCNKISessionParams{UserID: int64(user), SessionJSON: encrypted, QRUUID: row.qrUUID, Status: row.status, TokenExpiresAt: expires, CreatedAt: now})
		if err != nil {
			return err
		}
		row.generation = queryResult
		return nil
	}()
	if err != nil {
		return CnkiStatus{}, err
	}
	return summarizeCnki(&row, now), nil
}

// Clear stores a generation-advancing tombstone so delayed network results cannot resurrect credentials.
func (sessions *CnkiSessions) Clear(ctx context.Context, user identity.ID) (bool, error) {
	now := sessions.now()
	empty, err := sessions.codec.Encrypt("{}", secrets.CnkiContext(int64(user)))
	if err != nil {
		return false, err
	}
	var existed bool
	err = sessions.repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		err := func() error {
			queryResult, err := queries.HasCNKISession(ctx, connection, authdb.HasCNKISessionParams{UserID: int64(user)})
			if err != nil {
				return err
			}
			existed = queryResult
			return nil
		}()
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		_, err = queries.ClearCNKISession(ctx, connection, authdb.ClearCNKISessionParams{UserID: int64(user), SessionJSON: empty, CreatedAt: now})
		return err
	})
	return existed && err == nil, err
}

// TouchUsed records use only for a visible session without changing its generation.
func (sessions *CnkiSessions) TouchUsed(ctx context.Context, user identity.ID) (bool, error) {
	result, err := queries.TouchCNKISession(ctx, sessions.repository.database, authdb.TouchCNKISessionParams{LastUsedAt: new(sessions.now()), UserID: int64(user)})
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	return count > 0, err
}

func effectiveCnkiStatus(value any, row *cnkiRow, now float64) string {
	token := strings.TrimSpace(sessionString(value, "bff_user_token"))
	if token != "" {
		expires := jwtExpiration(token)
		if expires != nil && *expires <= now {
			return "expired"
		}
		return "active"
	}
	if strings.TrimSpace(row.qrUUID) != "" {
		return "waiting_scan"
	}
	status := strings.TrimSpace(row.status)
	if status == "" {
		return "empty"
	}
	return status
}

func summarizeCnki(row *cnkiRow, now float64) CnkiStatus {
	result := CnkiStatus{Status: "empty", CookieNames: []string{}}
	if row == nil {
		return result
	}
	value, _ := decodeSession(row.plaintext)
	token := strings.TrimSpace(sessionString(value, "bff_user_token"))
	result.HasBffUserToken = token != ""
	if token != "" {
		result.ExpiresAt = jwtExpiration(token)
	}
	if result.ExpiresAt != nil {
		remaining := math.Floor(math.Max(*result.ExpiresAt-now, 0))
		seconds := int64(math.MaxInt64)
		if remaining < float64(math.MaxInt64) {
			seconds = int64(remaining)
		}
		result.SecondsRemaining = &seconds
	}
	result.Status = effectiveCnkiStatus(value, row, now)
	result.Configured = result.Status != "empty"
	result.UpdatedAt = row.updated
	result.LastUsedAt = row.lastUsed
	if cookies, ok := sessionField(value, "cookies").([]any); ok {
		for _, cookie := range cookies {
			name := strings.TrimSpace(sessionString(cookie, "name"))
			if name != "" {
				result.CookieNames = append(result.CookieNames, name)
			}
		}
	}
	return result
}

// jwtExpiration reads only the unverified payload; authentication belongs to the upstream source.
func jwtExpiration(token string) *float64 {
	parts := strings.Split(token, ".")
	if len(parts) < 2 {
		return nil
	}
	output, isValid := decodeJwtPayload(parts[1])
	if !isValid {
		return nil
	}
	value, err := decodeSession(string(output))
	if err != nil {
		return nil
	}
	number, ok := sessionField(value, "exp").(json.Number)
	if !ok {
		return nil
	}
	expires, err := strconv.ParseFloat(string(number), 64)
	if err != nil {
		return nil
	}
	return &expires
}

// decodeJwtPayload preserves embedded padding and incomplete trailing sextets.
func decodeJwtPayload(payload string) ([]byte, bool) {
	var buffer uint32
	bits := uint8(0)
	output := []byte{}
	for _, character := range []byte(payload) {
		if character == '=' {
			continue
		}
		digit, isValid := jwtPayloadDigit(character)
		if !isValid {
			return nil, false
		}
		buffer = buffer<<6 | uint32(digit)
		bits += 6
		for bits >= 8 {
			bits -= 8
			output = append(output, byte(buffer>>bits))
		}
	}
	return output, true
}

// jwtPayloadDigit maps only the historical URL-safe base64 alphabet.
func jwtPayloadDigit(character byte) (byte, bool) {
	switch {
	case character >= 'A' && character <= 'Z':
		return character - 'A', true
	case character >= 'a' && character <= 'z':
		return character - 'a' + 26, true
	case character >= '0' && character <= '9':
		return character - '0' + 52, true
	case character == '-':
		return 62, true
	case character == '_':
		return 63, true
	default:
		return 0, false
	}
}
