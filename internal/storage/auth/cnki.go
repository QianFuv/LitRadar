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

	"github.com/QianFuv/LitRadar/internal/compat/jsonvalue"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
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
	QrUuid      string          `json:"-"`
	Status      string          `json:"-"`
	Generation  int64           `json:"-"`
}

func (data CnkiData) String() string       { return "CnkiData([REDACTED])" }
func (data CnkiData) GoString() string     { return data.String() }
func (data CnkiData) LogValue() slog.Value { return slog.StringValue(data.String()) }

// CnkiStatus is a credential-free lifecycle projection with explicit nullable timestamps.
type CnkiStatus struct {
	Configured       bool     `json:"configured"`
	Status           string   `json:"status"`
	HasBffUserToken  bool     `json:"has_bff_user_token"`
	ExpiresAt        *float64 `json:"expires_at"`
	SecondsRemaining *int64   `json:"seconds_remaining"`
	CookieNames      []string `json:"cookie_names"`
	UpdatedAt        *float64 `json:"updated_at"`
	LastUsedAt       *float64 `json:"last_used_at"`
}

type cnkiRow struct {
	plaintext, qrUuid, status string
	updated, lastUsed         *float64
	generation                int64
}

func (sessions *CnkiSessions) row(ctx context.Context, user identity.Id) (*cnkiRow, error) {
	var row cnkiRow
	err := sessions.repository.database.QueryRowContext(ctx, "SELECT session_json,qr_uuid,status,updated_at,last_used_at,generation FROM cnki_sessions WHERE user_id=? AND status<>'empty'", user).Scan(&row.plaintext, &row.qrUuid, &row.status, &row.updated, &row.lastUsed, &row.generation)
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
	if !jsonvalue.ValidJson(raw) {
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
func (sessions *CnkiSessions) Data(ctx context.Context, user identity.Id, isActiveOnly bool) (*CnkiData, error) {
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
	return &CnkiData{json.RawMessage(row.plaintext), row.qrUuid, status, row.generation}, nil
}

// Status tolerates malformed decrypted JSON while retaining effective token expiry rules.
func (sessions *CnkiSessions) Status(ctx context.Context, user identity.Id) (CnkiStatus, error) {
	row, err := sessions.row(ctx, user)
	if err != nil {
		return CnkiStatus{}, err
	}
	return summarizeCnki(row, sessions.now()), nil
}

// Reserve clears only the pending QR identifier on conflict, preserving an existing active session.
func (sessions *CnkiSessions) Reserve(ctx context.Context, user identity.Id) (int64, error) {
	empty, err := sessions.codec.Encrypt("{}", secrets.CnkiContext(int64(user)))
	if err != nil {
		return 0, err
	}
	var generation int64
	err = sessions.repository.database.QueryRowContext(ctx, `INSERT INTO cnki_sessions(user_id,session_json,qr_uuid,status,token_expires_at,created_at,updated_at,last_used_at,generation) VALUES(?1,?2,'','empty',NULL,?3,?3,NULL,1) ON CONFLICT(user_id) DO UPDATE SET qr_uuid='',generation=cnki_sessions.generation+1 RETURNING generation`, user, empty, sessions.now()).Scan(&generation)
	return generation, err
}

func (sessions *CnkiSessions) prepare(user identity.Id, data json.RawMessage, status string, qrUuid *string, now float64) (cnkiRow, string, *float64, error) {
	value, err := decodeSession(string(data))
	if err != nil {
		return cnkiRow{}, "", nil, err
	}
	plaintext, err := jsonvalue.EncodeJson(value)
	if err != nil {
		return cnkiRow{}, "", nil, err
	}
	resolved := ""
	if qrUuid != nil {
		resolved = strings.TrimSpace(*qrUuid)
	}
	if resolved == "" {
		resolved = strings.TrimSpace(sessionString(value, "qr_uuid"))
	}
	expires := jwtExpiration(sessionString(value, "bff_user_token"))
	encrypted, err := sessions.codec.Encrypt(plaintext, secrets.CnkiContext(int64(user)))
	return cnkiRow{plaintext: plaintext, qrUuid: resolved, status: status, updated: &now}, encrypted, expires, err
}

// Complete publishes only the matching generation and optional exact QR identity.
func (sessions *CnkiSessions) Complete(ctx context.Context, user identity.Id, generation int64, expectedQr *string, data json.RawMessage, status string, qrUuid *string) (*CnkiStatus, error) {
	now := sessions.now()
	row, encrypted, expires, err := sessions.prepare(user, data, status, qrUuid, now)
	if err != nil {
		return nil, err
	}
	result, err := sessions.repository.database.ExecContext(ctx, `UPDATE cnki_sessions SET session_json=?1,qr_uuid=?2,status=?3,token_expires_at=?4,updated_at=?5,generation=generation+1 WHERE user_id=?6 AND generation=?7 AND (?8 IS NULL OR qr_uuid=?8)`, encrypted, row.qrUuid, row.status, expires, now, user, generation, expectedQr)
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
func (sessions *CnkiSessions) Upsert(ctx context.Context, user identity.Id, data json.RawMessage, status string, qrUuid *string) (CnkiStatus, error) {
	now := sessions.now()
	row, encrypted, expires, err := sessions.prepare(user, data, status, qrUuid, now)
	if err != nil {
		return CnkiStatus{}, err
	}
	err = sessions.repository.database.QueryRowContext(ctx, `INSERT INTO cnki_sessions(user_id,session_json,qr_uuid,status,token_expires_at,created_at,updated_at,last_used_at,generation) VALUES(?1,?2,?3,?4,?5,?6,?6,NULL,1) ON CONFLICT(user_id) DO UPDATE SET session_json=excluded.session_json,qr_uuid=excluded.qr_uuid,status=excluded.status,token_expires_at=excluded.token_expires_at,updated_at=excluded.updated_at,generation=cnki_sessions.generation+1 RETURNING generation`, user, encrypted, row.qrUuid, row.status, expires, now).Scan(&row.generation)
	if err != nil {
		return CnkiStatus{}, err
	}
	return summarizeCnki(&row, now), nil
}

// Clear stores a generation-advancing tombstone so delayed network results cannot resurrect credentials.
func (sessions *CnkiSessions) Clear(ctx context.Context, user identity.Id) (bool, error) {
	now := sessions.now()
	empty, err := sessions.codec.Encrypt("{}", secrets.CnkiContext(int64(user)))
	if err != nil {
		return false, err
	}
	var existed bool
	err = sessions.repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		err := connection.QueryRowContext(ctx, "SELECT status<>'empty' FROM cnki_sessions WHERE user_id=?", user).Scan(&existed)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		_, err = connection.ExecContext(ctx, `INSERT INTO cnki_sessions(user_id,session_json,qr_uuid,status,token_expires_at,created_at,updated_at,last_used_at,generation) VALUES(?1,?2,'','empty',NULL,?3,?3,NULL,1) ON CONFLICT(user_id) DO UPDATE SET session_json=excluded.session_json,qr_uuid='',status='empty',token_expires_at=NULL,updated_at=excluded.updated_at,last_used_at=NULL,generation=cnki_sessions.generation+1`, user, empty, now)
		return err
	})
	return existed && err == nil, err
}

// TouchUsed records use only for a visible session without changing its generation.
func (sessions *CnkiSessions) TouchUsed(ctx context.Context, user identity.Id) (bool, error) {
	result, err := sessions.repository.database.ExecContext(ctx, "UPDATE cnki_sessions SET last_used_at=? WHERE user_id=? AND status<>'empty'", sessions.now(), user)
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
	if strings.TrimSpace(row.qrUuid) != "" {
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
	var buffer uint32
	bits := uint8(0)
	output := []byte{}
	for _, character := range []byte(parts[1]) {
		if character == '=' {
			continue
		}
		var digit byte
		switch {
		case character >= 'A' && character <= 'Z':
			digit = character - 'A'
		case character >= 'a' && character <= 'z':
			digit = character - 'a' + 26
		case character >= '0' && character <= '9':
			digit = character - '0' + 52
		case character == '-':
			digit = 62
		case character == '_':
			digit = 63
		default:
			return nil
		}
		buffer = buffer<<6 | uint32(digit)
		bits += 6
		for bits >= 8 {
			bits -= 8
			output = append(output, byte(buffer>>bits))
		}
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
