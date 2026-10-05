// Copyright 2026 LitRadar contributors. All rights reserved.
// Use of this source code is governed by the MIT-style license in the LICENSE file.

package mcp

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/modelcontextprotocol/go-sdk/internal/jsonrpc2"
	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
)

type litradarJsonScanner struct {
	body     []byte
	position int
}
type litradarJsonError struct{ detail string }

func (err *litradarJsonError) Error() string { return err.detail }

func (scanner *litradarJsonScanner) failure(message string, lookahead bool) error {
	position := scanner.position
	if lookahead && position < len(scanner.body) {
		position++
	}
	prefix := scanner.body[:position]
	line := bytes.Count(prefix, []byte{'\n'}) + 1
	column := position - (bytes.LastIndexByte(prefix, '\n') + 1)
	return &litradarJsonError{detail: fmt.Sprintf("%s at line %d column %d", message, line, column)}
}

func (scanner *litradarJsonScanner) whitespace() {
	for scanner.position < len(scanner.body) && bytes.ContainsRune([]byte(" \t\n\r"), rune(scanner.body[scanner.position])) {
		scanner.position++
	}
}

// ValidateLitRadarBody validates original JSON bytes with the legacy strict string/depth rules.
// Callers use it before normalization so error positions refer to the incoming body.
func ValidateLitRadarBody(body []byte) error {
	scanner := litradarJsonScanner{body: body}
	if err := scanner.value(0); err != nil {
		return err
	}
	scanner.whitespace()
	if scanner.position != len(body) {
		return scanner.failure("trailing characters", true)
	}
	return nil
}

func (scanner *litradarJsonScanner) value(depth int) error {
	scanner.whitespace()
	if scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing a value", false)
	}
	switch scanner.body[scanner.position] {
	case '{', '[':
		object := scanner.body[scanner.position] == '{'
		scanner.position++
		if depth+1 >= 128 {
			return scanner.failure("recursion limit exceeded", false)
		}
		end := byte(']')
		kind := "list"
		commaError := "expected `,` or `]`"
		if object {
			end = '}'
			kind = "object"
			commaError = "expected `,` or `}`"
		}
		scanner.whitespace()
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing "+map[bool]string{true: "an object", false: "a list"}[object], false)
		}
		if scanner.body[scanner.position] == end {
			scanner.position++
			return nil
		}
		for {
			if object {
				if scanner.position == len(scanner.body) {
					return scanner.failure("EOF while parsing a value", false)
				}
				if scanner.body[scanner.position] != '"' {
					return scanner.failure("key must be a string", true)
				}
				if err := scanner.stringValue(); err != nil {
					return err
				}
				scanner.whitespace()
				if scanner.position == len(scanner.body) {
					return scanner.failure("EOF while parsing an object", false)
				}
				if scanner.body[scanner.position] != ':' {
					return scanner.failure("expected `:`", true)
				}
				scanner.position++
			}
			if err := scanner.value(depth + 1); err != nil {
				return err
			}
			scanner.whitespace()
			if scanner.position == len(scanner.body) {
				article := "a "
				if object {
					article = "an "
				}
				return scanner.failure("EOF while parsing "+article+kind, false)
			}
			if scanner.body[scanner.position] == end {
				scanner.position++
				return nil
			}
			if scanner.body[scanner.position] != ',' {
				return scanner.failure(commaError, true)
			}
			scanner.position++
			scanner.whitespace()
			if scanner.position < len(scanner.body) && scanner.body[scanner.position] == end {
				return scanner.failure("trailing comma", true)
			}
		}
	case '"':
		return scanner.stringValue()
	case 't', 'f', 'n':
		literal := map[byte]string{'t': "true", 'f': "false", 'n': "null"}[scanner.body[scanner.position]]
		for _, character := range []byte(literal) {
			if scanner.position == len(scanner.body) {
				return scanner.failure("EOF while parsing a value", false)
			}
			if scanner.body[scanner.position] != character {
				return scanner.failure("expected ident", true)
			}
			scanner.position++
		}
		return nil
	default:
		if scanner.body[scanner.position] != '-' && (scanner.body[scanner.position] < '0' || scanner.body[scanner.position] > '9') {
			return scanner.failure("expected value", true)
		}
		start := scanner.position
		if scanner.body[scanner.position] == '-' {
			scanner.position++
		}
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing a value", false)
		}
		if scanner.body[scanner.position] == '0' {
			scanner.position++
			if scanner.digit() {
				return scanner.failure("invalid number", true)
			}
		} else {
			if !scanner.digit() {
				return scanner.failure("invalid number", true)
			}
			for scanner.digit() {
				scanner.position++
			}
		}
		if scanner.position < len(scanner.body) && scanner.body[scanner.position] == '.' {
			scanner.position++
			if scanner.position == len(scanner.body) {
				return scanner.failure("EOF while parsing a value", false)
			}
			if !scanner.digit() {
				return scanner.failure("invalid number", true)
			}
			for scanner.digit() {
				scanner.position++
			}
		}
		if scanner.position < len(scanner.body) && (scanner.body[scanner.position] == 'e' || scanner.body[scanner.position] == 'E') {
			mantissa := scanner.body[start:scanner.position]
			scanner.position++
			positiveExponent := true
			if scanner.position < len(scanner.body) && (scanner.body[scanner.position] == '+' || scanner.body[scanner.position] == '-') {
				positiveExponent = scanner.body[scanner.position] != '-'
				scanner.position++
			}
			if scanner.position == len(scanner.body) {
				return scanner.failure("EOF while parsing a value", false)
			}
			if !scanner.digit() {
				return scanner.failure("invalid number", true)
			}
			exponent := int64(0)
			nonzero := bytes.ContainsAny(mantissa, "123456789")
			for scanner.digit() {
				exponent = exponent*10 + int64(scanner.body[scanner.position]-'0')
				scanner.position++
				if exponent > 2147483647 {
					if nonzero && positiveExponent {
						return scanner.failure("number out of range", false)
					}
					for scanner.digit() {
						scanner.position++
					}
					return nil
				}
			}
		}
		if _, err := strconv.ParseFloat(string(scanner.body[start:scanner.position]), 64); err != nil {
			return scanner.failure("number out of range", true)
		}
		return nil
	}
}

func (scanner *litradarJsonScanner) digit() bool {
	return scanner.position < len(scanner.body) && scanner.body[scanner.position] >= '0' && scanner.body[scanner.position] <= '9'
}

func (scanner *litradarJsonScanner) hexUnit() (uint64, error) {
	start := scanner.position
	for range 4 {
		if scanner.position == len(scanner.body) {
			return 0, scanner.failure("EOF while parsing a string", false)
		}
		scanner.position++
	}
	for _, character := range scanner.body[start:scanner.position] {
		if !strings.ContainsRune("0123456789abcdefABCDEF", rune(character)) {
			return 0, scanner.failure("invalid escape", false)
		}
	}
	value, _ := strconv.ParseUint(string(scanner.body[start:scanner.position]), 16, 16)
	return value, nil
}

func (scanner *litradarJsonScanner) stringValue() error {
	scanner.position++
	start := scanner.position
	for scanner.position < len(scanner.body) {
		character := scanner.body[scanner.position]
		scanner.position++
		switch {
		case character == '"':
			if !utf8.Valid(scanner.body[start : scanner.position-1]) {
				return scanner.failure("invalid unicode code point", false)
			}
			return nil
		case character < 32:
			return scanner.failure("control character (\\u0000-\\u001F) found while parsing a string", false)
		case character == '\\':
			if scanner.position == len(scanner.body) {
				return scanner.failure("EOF while parsing a string", false)
			}
			escaped := scanner.body[scanner.position]
			scanner.position++
			if strings.ContainsRune(`"\/bfnrt`, rune(escaped)) {
				continue
			}
			if escaped != 'u' {
				return scanner.failure("invalid escape", false)
			}
			value, err := scanner.hexUnit()
			if err != nil {
				return err
			}
			if value >= 0xdc00 && value <= 0xdfff {
				return scanner.failure("lone leading surrogate in hex escape", false)
			}
			if value >= 0xd800 && value <= 0xdbff {
				for _, expected := range []byte{'\\', 'u'} {
					if scanner.position == len(scanner.body) {
						return scanner.failure("EOF while parsing a string", false)
					}
					actual := scanner.body[scanner.position]
					scanner.position++
					if actual != expected {
						return scanner.failure("unexpected end of hex escape", false)
					}
				}
				low, err := scanner.hexUnit()
				if err != nil {
					return err
				}
				if low < 0xdc00 || low > 0xdfff {
					return scanner.failure("lone leading surrogate in hex escape", false)
				}
			}
		}
	}
	return scanner.failure("EOF while parsing a string", false)
}

type litradarVersionsKey struct{}
type litradarVersions struct{ header, body string }

// WithLitRadarVersions retains original external labels across application version normalization.
// It only affects explicitly enabled compatibility mode and never trusts HTTP metadata for this context.
func WithLitRadarVersions(ctx context.Context, header, body string) context.Context {
	return context.WithValue(ctx, litradarVersionsKey{}, litradarVersions{header: header, body: body})
}

// decodeLitRadarMessage preserves signed integer identifiers without the upstream float64 conversion.
func decodeLitRadarMessage(body []byte) (message jsonrpc.Message, err error) {
	scanner := litradarJsonScanner{body: body}
	if err := scanner.value(0); err != nil {
		return nil, err
	}
	body = body[:scanner.position]
	defer func() {
		if err == nil {
			scanner.whitespace()
			if scanner.position != len(scanner.body) {
				message, err = nil, scanner.failure("trailing characters", true)
			}
		}
	}()
	var envelope map[string]json.RawMessage
	if err := json.Unmarshal(body, &envelope); err != nil {
		return nil, err
	}
	var version string
	if json.Unmarshal(envelope["jsonrpc"], &version) != nil || version != "2.0" {
		return nil, fmt.Errorf("invalid envelope")
	}
	raw := envelope["id"]
	var id jsonrpc.ID
	if len(raw) > 0 && raw[0] == '"' {
		var value string
		if err := json.Unmarshal(raw, &value); err != nil {
			return nil, err
		}
		id = jsonrpc2.StringID(value)
	} else {
		value, err := strconv.ParseInt(string(raw), 10, 64)
		if err == nil {
			id = jsonrpc2.Int64ID(value)
		}
	}
	var method string
	if rawMethod := envelope["method"]; len(rawMethod) > 0 && rawMethod[0] == '"' && json.Unmarshal(rawMethod, &method) == nil {
		return &jsonrpc.Request{ID: id, Method: method, Params: envelope["params"]}, nil
	}
	if result, exists := envelope["result"]; exists && id.IsValid() {
		return &jsonrpc.Response{ID: id, Result: result}, nil
	}
	var wire struct {
		Code    *int32
		Message *string
		Data    json.RawMessage
	}
	if json.Unmarshal(envelope["error"], &wire) == nil && wire.Code != nil && wire.Message != nil && (id.IsValid() || len(raw) == 0 || bytes.Equal(raw, []byte("null"))) {
		return &jsonrpc.Response{ID: id, Error: &jsonrpc.Error{Code: int64(*wire.Code), Message: *wire.Message, Data: wire.Data}}, nil
	}
	return nil, fmt.Errorf("invalid envelope")
}

// ValidateLitRadarMessage checks the original MCP envelope before application normalization.
// Envelope classification precedes the trailing-data check, as with the legacy typed reader.
func ValidateLitRadarMessage(body []byte) error {
	_, err := decodeLitRadarMessage(body)
	var syntax *litradarJsonError
	if err == nil || errors.As(err, &syntax) {
		return err
	}
	return errors.New("data did not match any variant of untagged enum JsonRpcMessage")
}

func litradarHTTPError(writer http.ResponseWriter, status int, message string) {
	writer.Header()["Content-Type"] = nil
	writer.WriteHeader(status)
	_, _ = io.WriteString(writer, message)
}

// prepareLitRadarRequest preserves legacy HTTP precedence while delegating all message ownership to the SDK.
func (handler *StreamableHTTPHandler) prepareLitRadarRequest(writer http.ResponseWriter, request *http.Request) (*http.Request, bool) {
	reject := func(status int, message string) (*http.Request, bool) {
		litradarHTTPError(writer, status, message)
		return nil, false
	}
	accept := request.Header.Get("Accept")
	switch request.Method {
	case http.MethodGet:
		if !strings.Contains(accept, "text/event-stream") {
			return reject(406, "Not Acceptable: Client must accept text/event-stream")
		}
	case http.MethodPost:
		if !strings.Contains(accept, "application/json") || !strings.Contains(accept, "text/event-stream") {
			return reject(406, "Not Acceptable: Client must accept both application/json and text/event-stream")
		}
		if !strings.HasPrefix(request.Header.Get("Content-Type"), "application/json") {
			return reject(415, "Unsupported Media Type: Content-Type must be application/json")
		}
	case http.MethodDelete:
	default:
		writer.Header().Set("Allow", "GET, POST, DELETE")
		return reject(405, "Method Not Allowed")
	}
	prepared := request.Clone(request.Context())
	prepared.Header.Set("Accept", "application/json, text/event-stream")
	prepared.Header.Set("Content-Type", "application/json")
	hasSession := len(request.Header.Values(sessionIDHeader)) > 0
	var message jsonrpc.Message
	if request.Method == http.MethodPost {
		body, err := io.ReadAll(request.Body)
		if err != nil {
			return reject(500, "Failed to read request body: "+err.Error())
		}
		message, err = decodeLitRadarMessage(body)
		if err != nil {
			detail := "data did not match any variant of untagged enum JsonRpcMessage"
			var syntaxError *litradarJsonError
			if errors.As(err, &syntaxError) {
				detail = syntaxError.Error()
			}
			return reject(415, "fail to deserialize request body "+detail)
		}
		prepared.Body = io.NopCloser(bytes.NewReader(body))
		if !hasSession {
			initialize, ok := message.(*jsonrpc.Request)
			if !ok || initialize.Method != methodInitialize || !initialize.IsCall() {
				return reject(422, "Unexpected message, expect initialize request")
			}
			var params InitializeParams
			if json.Unmarshal(initialize.Params, &params) != nil || params.Capabilities == nil || params.ClientInfo == nil {
				return reject(422, "Unexpected message, expect initialize request")
			}
			header, bodyVersion := request.Header.Get(protocolVersionHeader), params.ProtocolVersion
			if original, ok := request.Context().Value(litradarVersionsKey{}).(litradarVersions); ok {
				header, bodyVersion = original.header, original.body
			}
			if header != "" && header != bodyVersion {
				writeJSONRPCError(writer, 400, initialize.ID, &jsonrpc.Error{Code: jsonrpc.CodeInvalidRequest, Message: fmt.Sprintf("Invalid Request: MCP-Protocol-Version header (%s) does not match initialize params.protocolVersion (%s)", header, bodyVersion)})
				return nil, false
			}
		}
	} else if !hasSession {
		return reject(400, "Bad Request: Session ID is required")
	}
	if hasSession && request.Method != http.MethodDelete {
		handler.mu.Lock()
		exists := handler.sessions[request.Header.Get(sessionIDHeader)] != nil
		handler.mu.Unlock()
		if !exists {
			return reject(404, "Not Found: Session not found")
		}
	}
	version := request.Header.Get(protocolVersionHeader)
	if hasSession || request.Method != http.MethodPost {
		if version != "" {
			server := handler.getServer(request)
			known := false
			versions := supportedProtocolVersions
			if server != nil {
				versions = server.protocolVersions
			}
			for _, candidate := range versions {
				if candidate == version {
					known = true
					break
				}
			}
			if !known {
				return reject(400, "Bad Request: Unsupported MCP-Protocol-Version: "+version)
			}
		}
	}
	prepared = prepared.WithContext(context.WithValue(prepared.Context(), protocolVersionContextKey{}, version))
	return prepared, true
}

// CloseLitRadar closes all sessions and rejects new requests in compatibility mode.
// Callers must enable LitRadarCompatibility; the default SDK mode is unchanged.
func (h *StreamableHTTPHandler) CloseLitRadar() error {
	if !h.opts.LitRadarCompatibility {
		return fmt.Errorf("LitRadar compatibility is disabled")
	}
	h.closeAll()
	return nil
}

// resetLitRadarTimer changes only the existing session's timer, fencing callbacks already queued.
func (info *sessionInfo) resetLitRadarTimer(duration time.Duration, phase uint8) {
	info.timerMu.Lock()
	defer info.timerMu.Unlock()
	if info.litradarExpired {
		return
	}
	info.litradarPhase = phase
	info.litradarGeneration++
	generation := info.litradarGeneration
	if info.timer != nil {
		info.timer.Stop()
		info.timer = nil
	}
	if duration <= 0 {
		return
	}
	info.timer = time.AfterFunc(duration, func() {
		info.timerMu.Lock()
		if info.litradarExpired || info.litradarGeneration != generation {
			info.timerMu.Unlock()
			return
		}
		info.litradarExpired = true
		info.timer = nil
		info.timerMu.Unlock()
		if info.litradarRemove != nil {
			info.litradarRemove()
		}
		info.transport.connection.Close()
		info.session.Close()
	})
}

func (info *sessionInfo) readLitRadarInitialize() {
	info.timerMu.Lock()
	if info.litradarPhase == 0 && !info.litradarExpired {
		info.litradarPhase = 1
		info.litradarGeneration++
		if info.timer != nil {
			info.timer.Stop()
			info.timer = nil
		}
		info.timerMu.Unlock()
		return
	}
	info.timerMu.Unlock()
	info.touchLitRadar()
}

func (info *sessionInfo) touchLitRadar() {
	info.timerMu.Lock()
	ready := info.litradarPhase == 2 && !info.litradarExpired
	info.timerMu.Unlock()
	if ready {
		info.resetLitRadarTimer(info.timeout, 2)
	}
}

// litradarLease belongs to the existing SDK stream, never to a second session manager.
type litradarLease struct {
	writer  http.ResponseWriter
	context context.Context
	done    chan struct{}
}

func (lease *litradarLease) isLive() bool {
	if lease == nil || lease.context.Err() != nil {
		return false
	}
	select {
	case <-lease.done:
		return false
	default:
		return true
	}
}

type litradarEvent struct {
	index int
	data  []byte
	id    string
}

func writeLitRadarEvent(writer http.ResponseWriter, data []byte, id, retry string) error {
	return litradarWrite(writer, func(controller *http.ResponseController) error {
		if _, err := fmt.Fprintf(writer, "data: %s\n", data); err != nil {
			return err
		}
		if id != "" {
			if _, err := fmt.Fprintf(writer, "id: %s\n", id); err != nil {
				return err
			}
		}
		if retry != "" {
			if _, err := fmt.Fprintf(writer, "retry: %s\n", retry); err != nil {
				return err
			}
		}
		if _, err := fmt.Fprint(writer, "\n"); err != nil {
			return err
		}
		return controller.Flush()
	})
}

func litradarWrite(writer http.ResponseWriter, write func(*http.ResponseController) error) error {
	controller := http.NewResponseController(writer)
	if err := controller.SetWriteDeadline(time.Now().Add(10 * time.Second)); err != nil && !errors.Is(err, http.ErrNotSupported) {
		return err
	}
	defer controller.SetWriteDeadline(time.Time{})
	return write(controller)
}

func flushLitRadar(writer http.ResponseWriter) error {
	return litradarWrite(writer, func(controller *http.ResponseController) error { return controller.Flush() })
}

func keepaliveLitRadar(writer http.ResponseWriter) error {
	return litradarWrite(writer, func(controller *http.ResponseController) error {
		if _, err := fmt.Fprint(writer, ":\n\n"); err != nil {
			return err
		}
		return controller.Flush()
	})
}

// closeLitRadarLease requires the owning stream lock and tolerates SDK response completion.
func closeLitRadarLease(lease *litradarLease) {
	if lease == nil {
		return
	}
	select {
	case <-lease.done:
	default:
		close(lease.done)
	}
}

func (s *stream) releaseLitRadar(lease *litradarLease) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.litradarPrimary == lease {
		s.litradarPrimary = nil
		s.w = nil
		s.done = nil
	}
	for index, shadow := range s.litradarShadows {
		if shadow == lease {
			s.litradarShadows = append(s.litradarShadows[:index], s.litradarShadows[index+1:]...)
			break
		}
	}
}

func (c *streamableServerConn) hangLitRadar(ctx context.Context, s *stream, lease *litradarLease) {
	keepalive := time.NewTicker(15 * time.Second)
	defer keepalive.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-lease.done:
			return
		case <-c.done:
			return
		case <-keepalive.C:
			s.mu.Lock()
			select {
			case <-lease.done:
			default:
				if err := keepaliveLitRadar(lease.writer); err != nil {
					closeLitRadarLease(lease)
				}
			}
			s.mu.Unlock()
		}
	}
}

func (c *streamableServerConn) serveLitRadarGET(writer http.ResponseWriter, request *http.Request) {
	writer.Header().Set("Content-Type", "text/event-stream")
	writer.Header().Set("Cache-Control", "no-cache")
	streamID := ""
	index := 0
	hasResume := len(request.Header.Values(lastEventIDHeader)) > 0
	if hasResume {
		parts := strings.Split(request.Header.Get(lastEventIDHeader), "/")
		if len(parts) > 2 {
			return
		}
		parsed, err := strconv.ParseUint(parts[0], 10, 63)
		if err != nil {
			return
		}
		index = int(parsed)
		if len(parts) == 2 {
			parsedID, err := strconv.ParseUint(parts[1], 10, 64)
			if err != nil {
				return
			}
			streamID = strconv.FormatUint(parsedID, 10)
		}
	}
	if c.litradarSession != nil {
		c.litradarSession.touchLitRadar()
	}
	c.mu.Lock()
	s := c.streams[streamID]
	c.mu.Unlock()
	if s == nil {
		return
	}
	s.mu.Lock()
	if s.doneLocked() {
		s.mu.Unlock()
		return
	}
	lease := &litradarLease{writer: writer, context: request.Context(), done: make(chan struct{})}
	if streamID == "" && s.litradarPrimary.isLive() {
		live := s.litradarShadows[:0]
		for _, shadow := range s.litradarShadows {
			if shadow.isLive() {
				live = append(live, shadow)
			} else {
				closeLitRadarLease(shadow)
			}
		}
		s.litradarShadows = live
		if len(live) == 32 {
			closeLitRadarLease(live[0])
			s.litradarShadows = s.litradarShadows[1:]
		}
		s.litradarShadows = append(s.litradarShadows, lease)
		writer.WriteHeader(http.StatusOK)
		var writeError error
		if !hasResume {
			writeError = writeLitRadarEvent(writer, nil, "0", "3000")
		} else {
			writeError = flushLitRadar(writer)
		}
		if writeError != nil {
			closeLitRadarLease(lease)
		}
		s.mu.Unlock()
		defer s.releaseLitRadar(lease)
		c.hangLitRadar(request.Context(), s, lease)
		return
	}
	if hasResume && len(s.litradarCache) > 0 && index > s.litradarCache[len(s.litradarCache)-1].index+1 {
		s.mu.Unlock()
		return
	}
	closeLitRadarLease(s.litradarPrimary)
	s.litradarPrimary = lease
	s.w = writer
	s.done = lease.done
	writer.WriteHeader(http.StatusOK)
	var writeError error
	if !hasResume {
		writeError = writeLitRadarEvent(writer, nil, "0", "3000")
	}
	for _, event := range s.litradarCache {
		if writeError != nil {
			break
		}
		if event.index >= index {
			writeError = writeLitRadarEvent(writer, event.data, event.id, "")
		}
	}
	if writeError == nil {
		writeError = flushLitRadar(writer)
	}
	if writeError != nil {
		closeLitRadarLease(lease)
	}
	s.mu.Unlock()
	defer s.releaseLitRadar(lease)
	c.hangLitRadar(request.Context(), s, lease)
}
