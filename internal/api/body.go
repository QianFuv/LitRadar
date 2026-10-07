package api

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"

	wire "github.com/QianFuv/LitRadar/internal/domain/api"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
)

type bodyType struct {
	kind          string
	name          string
	fields        []bodyField
	element       *bodyType
	deniesUnknown bool
}
type bodyField struct {
	name       string
	value      bodyType
	hasDefault bool
	fallback   any
}

var stringBody = bodyType{kind: "string"}
var integerBody = bodyType{kind: "integer"}
var boolBody = bodyType{kind: "boolean"}
var articleBody = bodyType{kind: "article"}

func bodyFieldOf(name string, kind bodyType) bodyField { return bodyField{name: name, value: kind} }
func defaultBodyField(name string, kind bodyType, value any) bodyField {
	return bodyField{name, kind, true, value}
}
func structBody(name string, fields ...bodyField) bodyType {
	return bodyType{kind: "struct", name: name, fields: fields}
}
func listBody(element bodyType) bodyType { return bodyType{kind: "list", element: &element} }

func optionalBody(element bodyType) bodyType { return bodyType{kind: "optional", element: &element} }

var requestBodies = func() map[string]bodyType {
	favorite := structBody("FavoriteAdd", bodyFieldOf("article_id", articleBody), defaultBodyField("db_name", stringBody, ""), defaultBodyField("note", stringBody, ""))
	reference := structBody("FavoriteArticleRef", bodyFieldOf("article_id", articleBody), defaultBodyField("db_name", stringBody, ""))
	return map[string]bodyType{
		"ScheduledTaskCreate":        scheduledBody(false),
		"ScheduledTaskUpdate":        scheduledBody(true),
		"RuntimeSettingsUpdate":      runtimeSettingsBody(),
		"NotificationSettingsUpdate": notificationBody(),

		"AdminInviteCodeCreate":     {kind: "struct", name: "AdminInviteCodeCreate", deniesUnknown: true, fields: []bodyField{bodyFieldOf("expires_at", optionalBody(bodyType{kind: "float"})), bodyFieldOf("max_uses", optionalBody(integerBody))}},
		"LoginRequest":              structBody("LoginRequest", bodyFieldOf("username", stringBody), bodyFieldOf("password", stringBody)),
		"RegisterRequest":           structBody("RegisterRequest", bodyFieldOf("username", stringBody), bodyFieldOf("password", stringBody), bodyFieldOf("invite_code", stringBody)),
		"ChangePasswordRequest":     structBody("ChangePasswordRequest", bodyFieldOf("old_password", stringBody), bodyFieldOf("new_password", stringBody)),
		"TokenCreateRequest":        structBody("TokenCreateRequest", defaultBodyField("name", stringBody, ""), defaultBodyField("ttl", integerBody, int64(604800))),
		"FolderCreate":              structBody("FolderCreate", bodyFieldOf("name", stringBody), defaultBodyField("is_tracking", boolBody, false)),
		"FolderRename":              structBody("FolderRename", bodyFieldOf("name", stringBody)),
		"FavoriteAdd":               favorite,
		"FavoriteArticleRef":        reference,
		"FavoriteBatchCheckRequest": structBody("FavoriteBatchCheckRequest", bodyFieldOf("article_ids", listBody(articleBody)), defaultBodyField("db_name", stringBody, "")),
		"FavoriteBulkAdd":           structBody("FavoriteBulkAdd", bodyFieldOf("articles", listBody(favorite))),
		"FavoriteBulkRemove":        structBody("FavoriteBulkRemove", bodyFieldOf("articles", listBody(reference))),
		"FavoriteBulkMove":          structBody("FavoriteBulkMove", bodyFieldOf("target_folder_id", integerBody), bodyFieldOf("articles", listBody(reference))),
		"TrackingSetRequest":        structBody("TrackingSetRequest", bodyFieldOf("folder_id", integerBody)),
	}
}()

// extractBody enforces optional-header admission before buffering and typed decoding.
func extractBody(request *http.Request, kind bodyType, isOptional bool) (any, *apiError) {
	contentTypes, hasType := request.Header["Content-Type"]
	if isOptional && !hasType {
		return nil, nil
	}
	contentType := ""
	if len(contentTypes) > 0 {
		contentType = contentTypes[0]
	}
	if !isJsonContentType(contentType) {
		return nil, &apiError{status: 415, detail: "Expected request with `Content-Type: application/json`", isPlain: true}
	}
	body, failure := bufferRequestBody(request)
	if failure != nil {
		return nil, failure
	}
	scanner := bodyScanner{body: body}
	value, err := scanner.typed(kind, "", 0)
	if err == nil && (scanner.peek() != 0 || scanner.position < len(scanner.body)) {
		err = scanner.failure("trailing characters", true)
	}
	if err == nil {
		return value, nil
	}
	return nil, bodyRejection(err.(*bodyFailure))
}

// bufferRequestBody preserves length-limit precedence over reader errors.
func bufferRequestBody(request *http.Request) ([]byte, *apiError) {
	const limit = 2 * 1024 * 1024
	body, err := io.ReadAll(io.LimitReader(request.Body, limit+1))
	if len(body) > limit {
		return nil, &apiError{status: 413, detail: "Failed to buffer the request body: length limit exceeded", isPlain: true}
	}
	if err != nil {
		return nil, &apiError{status: 400, detail: "Failed to buffer the request body: " + err.Error(), isPlain: true}
	}
	return body, nil
}

// bodyRejection distinguishes syntax and typed data failures without changing public details.
func bodyRejection(failure *bodyFailure) *apiError {
	status, prefix := 400, "Failed to parse the request body as JSON: "
	if failure.isData {
		status, prefix = 422, "Failed to deserialize the JSON body into the target type: "
	}
	if failure.path != "" {
		prefix += failure.path + ": "
	}
	return &apiError{status: status, detail: prefix + failure.detail, isPlain: true}
}

func (scanner *bodyScanner) dataFailure(message string) error {
	failure := scanner.failure(message, false).(*bodyFailure)
	failure.isData = true
	return failure
}

// typed attaches the innermost available path to failures from the custom decoder.
func (scanner *bodyScanner) typed(kind bodyType, path string, depth int) (value any, err error) {
	defer func() {
		if err != nil {
			failure := err.(*bodyFailure)
			if failure.path == "" {
				failure.path = path
			}
		}
	}()
	token := scanner.peek()
	if token == 0 && scanner.position == len(scanner.body) {
		return nil, scanner.failure("EOF while parsing a value", false)
	}
	return scanner.decodeTyped(kind, path, depth, token)
}

// decodeTyped handles raw, tagged and optional values before collection or scalar decoding.
func (scanner *bodyScanner) decodeTyped(kind bodyType, path string, depth int, token byte) (any, error) {
	switch kind.kind {
	case "raw":
		return scanner.rawBody()
	case "job":
		return scanner.job(path, depth)
	case "optional":
		if token == 'n' {
			return nil, scanner.literal("null")
		}
		return scanner.typed(*kind.element, path, depth)
	default:
		return scanner.collectionOrScalar(kind, path, depth, token)
	}
}

// rawBody consumes ignored content without imposing typed recursion or Unicode restrictions.
func (scanner *bodyScanner) rawBody() (any, error) {
	start := scanner.position
	if err := scanner.skip(); err != nil {
		return nil, err
	}
	return json.RawMessage(scanner.body[start:scanner.position]), nil
}

// collectionOrScalar recognizes accepted collection shapes before reporting scalar mismatches.
func (scanner *bodyScanner) collectionOrScalar(kind bodyType, path string, depth int, token byte) (any, error) {
	if kind.kind == "map" && token == '{' {
		return scanner.dictionary(*kind.element, path, depth)
	}
	if kind.kind == "struct" && (token == '{' || token == '[') {
		scanner.position++
		if depth+1 >= 128 {
			return nil, scanner.failure("recursion limit exceeded", false)
		}
		return scanner.structure(kind, path, depth+1, token == '[')
	}
	if kind.kind == "list" && token == '[' {
		return scanner.typedList(*kind.element, path, depth)
	}
	return scanner.scalar(kind, token)
}

// typedList checks its depth after the opener and retains per-element paths and separators.
func (scanner *bodyScanner) typedList(element bodyType, path string, depth int) (any, error) {
	scanner.position++
	if depth+1 >= 128 {
		return nil, scanner.failure("recursion limit exceeded", false)
	}
	values := []any{}
	for {
		if scanner.peek() == ']' {
			scanner.position++
			return values, nil
		}
		if scanner.position == len(scanner.body) {
			return nil, scanner.failure("EOF while parsing a list", false)
		}
		if err := scanner.listSeparator(len(values)); err != nil {
			return nil, err
		}
		item, err := scanner.typed(element, fmt.Sprintf("%s[%d]", path, len(values)), depth+1)
		if err != nil {
			return nil, err
		}
		values = append(values, item)
	}
}

// listSeparator retains the distinct EOF after an element separator.
func (scanner *bodyScanner) listSeparator(count int) error {
	if count == 0 {
		return nil
	}
	if err := scanner.separator(']'); err != nil {
		return err
	}
	if scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing a value", false)
	}
	return nil
}

// scalar converts supported scalar forms and rejects mismatched collections without traversing them.
func (scanner *bodyScanner) scalar(kind bodyType, token byte) (any, error) {
	switch token {
	case '"':
		return scanner.stringBodyValue(kind)
	case 't', 'f', 'n':
		return scanner.literalBodyValue(kind, token)
	case '{', '[':
		return nil, scanner.typeFailure(kind, map[byte]string{'{': "map", '[': "sequence"}[token])
	default:
		return scanner.numericBodyValue(kind, token)
	}
}

// stringBodyValue decodes a validated string or converts a decimal article identity.
func (scanner *bodyScanner) stringBodyValue(kind bodyType) (any, error) {
	start := scanner.position
	if err := scanner.stringValue(); err != nil {
		return nil, err
	}
	var text string
	_ = json.Unmarshal(scanner.body[start:scanner.position], &text)
	if kind.kind == "string" {
		return text, nil
	}
	if kind.kind == "article" {
		return scanner.articleString(text)

	}
	return nil, scanner.typeFailure(kind, "string "+wire.DebugString(text))
}

// articleString retains decimal-string integer diagnostics for article identities.
func (scanner *bodyScanner) articleString(text string) (any, error) {
	number, parseErr := strconv.ParseInt(text, 10, 64)
	if parseErr == nil {
		return number, nil
	}
	detail := "invalid digit found in string"
	if text == "" {
		detail = "cannot parse integer from empty string"
	} else if parseErr.(*strconv.NumError).Err == strconv.ErrRange {
		if strings.HasPrefix(text, "-") {
			detail = "number too small to fit in target type"
		} else {
			detail = "number too large to fit in target type"
		}
	}
	return nil, scanner.dataFailure(detail)
}

// literalBodyValue decodes boolean and null tokens before typed rejection.
func (scanner *bodyScanner) literalBodyValue(kind bodyType, token byte) (any, error) {
	literal := map[byte]string{'t': "true", 'f': "false", 'n': "null"}[token]
	if err := scanner.literal(literal); err != nil {
		return nil, err
	}
	if token != 'n' && kind.kind == "boolean" {
		return token == 't', nil
	}
	actual := "null"
	if token != 'n' {
		actual = "boolean `" + literal + "`"
	}
	return nil, scanner.typeFailure(kind, actual)
}

// numericBodyValue scans a JSON number before preserving signed, unsigned and float conversions.
func (scanner *bodyScanner) numericBodyValue(kind bodyType, token byte) (any, error) {
	start := scanner.position
	if !bodyNumberStart(token) {
		return nil, scanner.failure("expected value", true)
	}
	if err := scanner.number(); err != nil {
		return nil, err
	}
	number, parseErr := sources.ParseNumber(json.Number(scanner.body[start:scanner.position]))
	if parseErr != nil {
		return nil, scanner.failure("number out of range", false)
	}
	if kind.kind == "float" {
		return number.AsFloat64(), nil
	}
	if unsigned, ok := number.AsUint64(); ok && kind.kind == "unsigned" {
		return unsigned, nil
	}
	if signed, ok := number.AsInt64(); ok {
		return scanner.signedBodyNumber(kind, number, signed)
	}
	if unsigned, ok := number.AsUint64(); ok {
		return scanner.unsignedBodyNumber(kind, number, unsigned)
	}
	return nil, scanner.typeFailure(kind, "floating point `"+number.String()+"`")
}

// bodyNumberStart recognizes the sign and decimal digits allowed to start a JSON number.
func bodyNumberStart(token byte) bool {
	return token == '-' || token >= '0' && token <= '9'
}

// signedBodyNumber preserves unsigned rejection and signed article conversion.
func (scanner *bodyScanner) signedBodyNumber(kind bodyType, number sources.Number, signed int64) (any, error) {
	if kind.kind == "unsigned" {
		expected := "u64"
		if kind.name != "" {
			expected = kind.name
		}
		return nil, scanner.dataFailure(fmt.Sprintf("invalid value: integer `%d`, expected %s", signed, expected))
	}
	if kind.kind == "integer" || kind.kind == "article" {
		return signed, nil
	}
	return nil, scanner.typeFailure(kind, "integer `"+number.String()+"`")
}

// unsignedBodyNumber reports integral overflow before the ordinary type mismatch.
func (scanner *bodyScanner) unsignedBodyNumber(kind bodyType, number sources.Number, unsigned uint64) (any, error) {
	if kind.kind == "article" {
		return nil, scanner.dataFailure("out of range integral type conversion attempted")
	}
	if kind.kind == "integer" {
		return nil, scanner.dataFailure(fmt.Sprintf("invalid value: integer `%d`, expected i64", unsigned))
	}
	return nil, scanner.typeFailure(kind, "integer `"+number.String()+"`")
}

// typeFailure builds the exact expected-type description for a decoded scalar mismatch.
func (scanner *bodyScanner) typeFailure(kind bodyType, actual string) error {
	expected := map[string]string{"string": "a string", "integer": "i64", "unsigned": "u64", "float": "f64", "boolean": "a boolean", "article": "a SQLite integer or decimal string", "list": "a sequence", "map": "a map", "struct": "struct " + kind.name}[kind.kind]
	if kind.kind == "unsigned" && kind.name != "" {
		expected = kind.name
	}
	return scanner.dataFailure("invalid type: " + actual + ", expected " + expected)
}

func (scanner *bodyScanner) separator(end byte) error {
	token := scanner.peek()
	if token == 0 && scanner.position == len(scanner.body) {
		kind := "a list"
		if end == '}' {
			kind = "an object"
		}
		return scanner.failure("EOF while parsing "+kind, false)
	}
	if token != ',' {
		return scanner.failure(fmt.Sprintf("expected `,` or `%c`", end), true)
	}
	scanner.position++
	if scanner.peek() == end {
		return scanner.failure("trailing comma", true)
	}
	return nil
}

// structure decodes positional or object fields before applying missing-field rules.
func (scanner *bodyScanner) structure(kind bodyType, path string, depth int, isSequence bool) (any, error) {
	result := map[string]any{}
	end := byte('}')
	if isSequence {
		end = ']'
	}
	count := 0
	for scanner.peek() != end {
		if err := scanner.structureEntry(kind, path, depth, isSequence, end, count, result); err != nil {
			return nil, err
		}
		count++
	}
	scanner.position++
	if err := scanner.completeStructure(kind, isSequence, count, result); err != nil {
		return nil, err
	}
	return result, nil
}

// structureEntry preserves positional bounds, separators and object-key admission order.
func (scanner *bodyScanner) structureEntry(kind bodyType, path string, depth int, isSequence bool, end byte, count int, result map[string]any) error {
	if isSequence {
		if err := scanner.sequenceFieldStart(kind, count); err != nil {
			return err
		}
	}
	if count > 0 {
		if err := scanner.separator(end); err != nil {
			return err
		}
	}
	if isSequence && count >= len(kind.fields) {
		return scanner.failure("trailing characters", true)
	}
	fieldIndex, unknownKey := count, ""
	if !isSequence {
		index, key, err := scanner.structureObjectField(kind, path, count, result)
		if err != nil {
			return err
		}
		fieldIndex, unknownKey = index, key
	}
	if fieldIndex < 0 {
		return scanner.ignoreStructureField(unknownKey, path)
	}
	return scanner.decodeStructureField(kind.fields[fieldIndex], fieldIndex, path, depth, isSequence, end, result)
}

// sequenceFieldStart rejects excess elements before separator parsing and retains EOF precedence.
func (scanner *bodyScanner) sequenceFieldStart(kind bodyType, count int) error {
	if count >= len(kind.fields) {
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing a list", false)
		}
		if scanner.peek() == ',' {
			scanner.position++
			if scanner.peek() == ']' {
				return scanner.failure("trailing comma", true)
			}
		}
		return scanner.failure("trailing characters", true)
	}
	if scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing a list", false)
	}
	return nil
}

// structureObjectField resolves a key and rejects duplicate or denied keys before its colon.
func (scanner *bodyScanner) structureObjectField(kind bodyType, path string, count int, result map[string]any) (int, string, error) {
	key, err := scanner.structureKey(count)
	if err != nil {
		return -1, "", err
	}
	fieldIndex := -1
	for index, field := range kind.fields {
		if field.name == key {
			fieldIndex = index
			break
		}
	}
	if err := scanner.admitStructureKey(kind, key, path, fieldIndex, result); err != nil {
		return -1, key, err
	}
	if scanner.peek() == 0 && scanner.position == len(scanner.body) {
		return -1, key, scanner.failure("EOF while parsing an object", false)
	}
	if scanner.peek() != ':' {
		return -1, key, scanner.failure("expected `:`", true)
	}
	scanner.position++
	return fieldIndex, key, nil
}

// structureKey retains the distinct empty-object and post-separator EOF diagnostics.
func (scanner *bodyScanner) structureKey(count int) (string, error) {
	if scanner.peek() == 0 && scanner.position == len(scanner.body) {
		if count > 0 {
			return "", scanner.failure("EOF while parsing a value", false)
		}
		return "", scanner.failure("EOF while parsing an object", false)
	}
	if scanner.peek() != '"' {
		return "", scanner.failure("key must be a string", true)
	}
	start := scanner.position
	if err := scanner.stringValue(); err != nil {
		return "", err
	}
	var key string
	_ = json.Unmarshal(scanner.body[start:scanner.position], &key)
	return key, nil
}

// admitStructureKey retains early data rejection without validating the field value.
func (scanner *bodyScanner) admitStructureKey(kind bodyType, key, path string, fieldIndex int, result map[string]any) error {
	if _, exists := result[key]; exists {
		scanner.rejectedStructureKey()
		return scanner.dataFailure("duplicate field `" + key + "`")
	}
	if fieldIndex < 0 && kind.deniesUnknown {
		scanner.rejectedStructureKey()
		return scanner.unknownStructureField(kind, key, path)
	}
	return nil
}

// rejectedStructureKey consumes a directly following closing brace before positioning the failure.
func (scanner *bodyScanner) rejectedStructureKey() {
	scanner.whitespace()
	if scanner.peek() == '}' {
		scanner.position++
	}
}

// unknownStructureField preserves declared-name ordering and the rejected key's path.
func (scanner *bodyScanner) unknownStructureField(kind bodyType, key, path string) error {
	names := make([]string, len(kind.fields))
	for index, field := range kind.fields {
		names[index] = "`" + field.name + "`"
	}
	expected := strings.Join(names, " or ")
	if len(names) > 2 {
		expected = "one of " + strings.Join(names, ", ")
	}
	failure := scanner.dataFailure("unknown field `" + key + "`, expected " + expected).(*bodyFailure)
	failure.path = objectFieldPath(path, key)
	return failure
}

// objectFieldPath joins object names while retaining an empty container path.
func objectFieldPath(path, key string) string {
	if path != "" {
		return path + "." + key
	}
	return key
}

// ignoreStructureField assigns ignored-content syntax failures to the unknown field.
func (scanner *bodyScanner) ignoreStructureField(key, path string) error {
	if err := scanner.skip(); err != nil {
		err.(*bodyFailure).path = objectFieldPath(path, key)
		return err
	}
	return nil
}

// decodeStructureField retains object/sequence paths and positions delayed tagged-job errors.
func (scanner *bodyScanner) decodeStructureField(field bodyField, index int, path string, depth int, isSequence bool, end byte, result map[string]any) error {
	childPath := objectFieldPath(path, field.name)
	if isSequence {
		childPath = fmt.Sprintf("%s[%d]", path, index)
	}
	value, err := scanner.typed(field.value, childPath, depth)
	if err != nil {
		scanner.positionStructureFailure(err.(*bodyFailure), isSequence, end)
		return err
	}
	result[field.name] = value
	return nil
}

// positionStructureFailure resolves delayed errors using the enclosing input rather than reparsed bytes.
func (scanner *bodyScanner) positionStructureFailure(failure *bodyFailure, isSequence bool, end byte) {
	if !failure.needsPosition {
		return
	}
	token := scanner.peek()
	if token == end {
		scanner.position++
	} else if isSequence && token == ',' {
		scanner.position++
		scanner.peek()
	}
	positioned := scanner.failure(failure.detail, false).(*bodyFailure)
	failure.detail, failure.needsPosition = positioned.detail, false
}

// completeStructure applies defaults and object-only missing optional values in declaration order.
func (scanner *bodyScanner) completeStructure(kind bodyType, isSequence bool, count int, result map[string]any) error {
	for _, field := range kind.fields {
		if _, exists := result[field.name]; exists {
			continue
		}
		if field.hasDefault {
			result[field.name] = field.fallback
			continue
		}
		if !isSequence && field.value.kind == "optional" {
			result[field.name] = nil
			continue
		}
		if isSequence {
			return scanner.dataFailure(fmt.Sprintf("invalid length %d, expected struct %s with %d elements", count, kind.name, len(kind.fields)))
		}
		return scanner.dataFailure("missing field `" + field.name + "`")
	}
	return nil
}

// skip consumes ignored values iteratively, preserving serde's relaxed strings and numbers.
func (scanner *bodyScanner) skip() error {
	scanner.isIgnoring = true
	defer func() { scanner.isIgnoring = false }()
	stack := []byte{'v'}
	for len(stack) > 0 {
		action := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		next, err := scanner.skipAction(action, stack)
		if err != nil {
			return err
		}
		stack = next
	}
	return nil
}

// skipAction advances one iterative ignored-value stack production.
func (scanner *bodyScanner) skipAction(action byte, stack []byte) ([]byte, error) {
	token := scanner.peek()
	switch action {
	case 'v':
		return scanner.skipValue(token, stack)
	case 'a', 'A':
		return scanner.skipArray(action, token, stack)
	case 'o', 'O':
		return scanner.skipObject(action, token, stack)
	}
	return stack, nil
}

// skipValue consumes an ignored scalar or pushes its collection continuation.
func (scanner *bodyScanner) skipValue(token byte, stack []byte) ([]byte, error) {
	switch token {
	case '{':
		scanner.position++
		stack = append(stack, 'o')
	case '[':
		scanner.position++
		stack = append(stack, 'a')
	default:
		return stack, scanner.skipScalar(token)
	}
	return stack, nil
}

// skipScalar retains ignored-value grammar checks while delegating relaxed scalar scanning.
func (scanner *bodyScanner) skipScalar(token byte) error {
	switch token {
	case 0:
		if scanner.position < len(scanner.body) {
			return scanner.failure("expected value", true)
		}
		return scanner.failure("EOF while parsing a value", false)
	case '"':
		if err := scanner.stringValue(); err != nil {
			return err
		}
	case 't', 'f', 'n':
		if err := scanner.literal(map[byte]string{'t': "true", 'f': "false", 'n': "null"}[token]); err != nil {
			return err
		}
	default:
		if !bodyNumberStart(token) {
			return scanner.failure("expected value", true)
		}
		if err := scanner.number(); err != nil {
			return err
		}
	}
	return nil
}

// skipArray preserves the ignored-value trailing-comma diagnostic substitution.
func (scanner *bodyScanner) skipArray(action, token byte, stack []byte) ([]byte, error) {
	if token == ']' {
		scanner.position++
		return stack, nil
	}
	if action == 'A' {
		if err := scanner.separator(']'); err != nil {
			if scanner.peek() == ']' {
				return stack, scanner.failure("expected value", true)
			}
			return stack, err
		}
	}
	stack = append(stack, 'A', 'v')
	return stack, nil
}

// skipObject retains iterative key/value traversal and object separator errors.
func (scanner *bodyScanner) skipObject(action, token byte, stack []byte) ([]byte, error) {
	if token == '}' {
		scanner.position++
		return stack, nil
	}
	if action == 'O' {
		if err := scanner.separator('}'); err != nil {
			if scanner.peek() == '}' {
				return stack, scanner.failure("key must be a string", true)
			}
			return stack, err
		}
	}
	if err := scanner.skipObjectKey(); err != nil {
		return stack, err
	}
	stack = append(stack, 'O', 'v')
	return stack, nil
}

// skipObjectKey validates an ignored object's key and colon at their original positions.
func (scanner *bodyScanner) skipObjectKey() error {
	if scanner.peek() == 0 && scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing an object", false)
	}
	if scanner.peek() != '"' {
		return scanner.failure("key must be a string", true)
	}
	if err := scanner.stringValue(); err != nil {
		return err
	}
	if scanner.peek() == 0 && scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing an object", false)
	}
	if scanner.peek() != ':' {
		return scanner.failure("expected `:`", true)
	}
	scanner.position++
	return nil
}
