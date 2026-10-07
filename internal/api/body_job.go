package api

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
)

func scheduledBody(isUpdate bool) bodyType {
	job := bodyType{kind: "job"}
	unsigned := bodyType{kind: "unsigned"}
	kind := structBody("ScheduledTaskCreate", bodyFieldOf("name", stringBody), bodyFieldOf("job", job), bodyFieldOf("cron", stringBody), defaultBodyField("timezone", stringBody, "UTC"), defaultBodyField("timeout_seconds", unsigned, uint64(3600)), defaultBodyField("coalesce", boolBody, true), defaultBodyField("enabled", boolBody, true))
	if isUpdate {
		kind = structBody("ScheduledTaskUpdate", bodyFieldOf("name", optionalBody(stringBody)), bodyFieldOf("job", optionalBody(job)), bodyFieldOf("cron", optionalBody(stringBody)), bodyFieldOf("timezone", optionalBody(stringBody)), bodyFieldOf("timeout_seconds", optionalBody(unsigned)), bodyFieldOf("coalesce", optionalBody(boolBody)), bodyFieldOf("enabled", optionalBody(boolBody)))
	}
	kind.deniesUnknown = true
	return kind
}

// content scans strict raw payloads and assigns the innermost error path.
func (scanner *bodyScanner) content(path string, depth int) (raw json.RawMessage, err error) {
	defer func() {
		if err != nil {
			failure := err.(*bodyFailure)
			if failure.path == "" {
				failure.path = path
			}
		}
	}()
	token := scanner.peek()
	start := scanner.position
	if scanner.position == len(scanner.body) {
		return nil, scanner.failure("EOF while parsing a value", false)
	}
	switch token {
	case '{', '[':
		err = scanner.contentCollection(token, path, depth)
	case '"':
		err = scanner.stringValue()
	case 'n':
		err = scanner.literal("null")
	case 't':
		err = scanner.literal("true")
	case 'f':
		err = scanner.literal("false")
	default:
		err = scanner.contentNumber(token, start)
	}
	if err != nil {
		return nil, err
	}
	return scanner.body[start:scanner.position], nil
}

// contentCollection scans strict nested content before delayed tagged-job validation.
func (scanner *bodyScanner) contentCollection(token byte, path string, depth int) error {
	scanner.position++
	if depth+1 >= 128 {
		return scanner.failure("recursion limit exceeded", false)
	}
	end, collection := byte('}'), "an object"
	if token == '[' {
		end, collection = ']', "a list"
	}
	count := 0
	for scanner.peek() != end {
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing "+collection, false)
		}
		if count > 0 {
			if err := scanner.separator(end); err != nil {
				return err
			}
		}
		childPath, err := scanner.contentChildPath(token, path, count)
		if err != nil {
			return err
		}
		if _, err := scanner.content(childPath, depth+1); err != nil {
			return err
		}
		count++
	}
	scanner.position++
	return nil
}

// contentChildPath consumes an object key or retains the sequence element path.
func (scanner *bodyScanner) contentChildPath(token byte, path string, count int) (string, error) {
	childPath := fmt.Sprintf("%s[%d]", path, count)
	if token == '{' {
		if scanner.peek() != '"' {
			return "", scanner.failure("key must be a string", true)
		}
		keyStart := scanner.position
		if err := scanner.stringValue(); err != nil {
			return "", err
		}
		var key string
		_ = json.Unmarshal(scanner.body[keyStart:scanner.position], &key)
		childPath = path + "." + key
		if scanner.peek() != ':' {
			return "", scanner.failure("expected `:`", true)
		}
		scanner.position++
	}
	return childPath, nil
}

// contentNumber preserves strict numeric range validation of buffered job payloads.
func (scanner *bodyScanner) contentNumber(token byte, start int) error {
	if token != '-' && (token < '0' || token > '9') {
		return scanner.failure("expected value", true)
	}
	err := scanner.number()
	if err == nil {
		if _, numberErr := sources.ParseNumber(json.Number(scanner.body[start:scanner.position])); numberErr != nil {
			err = scanner.failure("number out of range", false)
		}
	}
	return err
}

type jobBodyState struct {
	name       string
	parts      []string
	path       string
	depth      int
	count      int
	isSequence bool
	end        byte
}

// job reads the original tagged content before reparsing its variant fields.
func (scanner *bodyScanner) job(path string, depth int) (any, error) {
	token := scanner.peek()
	if token != '{' && token != '[' {
		return scanner.jobTypeFailure(path, depth)
	}
	scanner.position++
	if depth+1 >= 128 {
		return nil, scanner.failure("recursion limit exceeded", false)
	}
	state := jobBodyState{parts: []string{}, path: path, depth: depth, isSequence: token == '[', end: '}'}
	if state.isSequence {
		state.end = ']'
	}
	if err := scanner.collectJob(&state); err != nil {
		return nil, err
	}
	if state.name == "" {
		return nil, scanner.dataFailure("missing field `kind`")
	}
	return decodeJobVariant(state)
}

// jobTypeFailure preserves the internally tagged enum description on scalar mismatches.
func (scanner *bodyScanner) jobTypeFailure(path string, depth int) (any, error) {
	_, err := scanner.typed(structBody("ScheduledJobSpec"), path, depth)
	if err != nil {
		failure := err.(*bodyFailure)
		failure.detail = strings.Replace(failure.detail, "struct ScheduledJobSpec", "internally tagged enum ScheduledJobSpec", 1)
	}
	return nil, err
}

// collectJob scans all original payload fields using the original collection separators.
func (scanner *bodyScanner) collectJob(state *jobBodyState) error {
	for scanner.peek() != state.end {
		if scanner.position == len(scanner.body) {
			collection := "an object"
			if state.isSequence {
				collection = "a list"
			}
			return scanner.failure("EOF while parsing "+collection, false)
		}
		if state.count > 0 {
			if err := scanner.separator(state.end); err != nil {
				return err
			}
		}
		if err := scanner.jobEntry(state); err != nil {
			return err
		}
		state.count++
	}
	scanner.position++
	return nil
}

// jobEntry interprets the tag immediately and buffers other strictly scanned fields.
func (scanner *bodyScanner) jobEntry(state *jobBodyState) error {
	key, encodedKey := "", ""
	childPath := fmt.Sprintf("%s[%d]", state.path, state.count)
	if !state.isSequence {
		parsedKey, rawKey, parsedPath, err := scanner.jobObjectKey(state)
		if err != nil {
			return err
		}
		key, encodedKey, childPath = parsedKey, rawKey, parsedPath
	}
	if key == "kind" || state.isSequence && state.count == 0 {
		return scanner.jobTag(state, childPath)
	}
	raw, err := scanner.content(childPath, state.depth+1)
	if err != nil {
		return err
	}
	if state.isSequence {
		state.parts = append(state.parts, string(raw))
	} else {
		state.parts = append(state.parts, encodedKey+":"+string(raw))
	}
	return nil
}

// jobObjectKey rejects a duplicate nonempty tag before checking its colon.
func (scanner *bodyScanner) jobObjectKey(state *jobBodyState) (string, string, string, error) {
	key, encodedKey, childPath := "", "", ""
	if scanner.peek() != '"' {
		return "", "", "", scanner.failure("key must be a string", true)
	}
	start := scanner.position
	if err := scanner.stringValue(); err != nil {
		return "", "", "", err
	}
	encodedKey = string(scanner.body[start:scanner.position])
	_ = json.Unmarshal([]byte(encodedKey), &key)
	childPath = state.path + "." + key
	if key == "kind" && state.name != "" {
		scanner.peek()
		if scanner.peek() == '}' {
			scanner.position++
		}
		return "", "", "", scanner.dataFailure("duplicate field `kind`")
	}
	if scanner.peek() != ':' {
		return "", "", "", scanner.failure("expected `:`", true)
	}
	scanner.position++
	return key, encodedKey, childPath, nil
}

// jobTag decodes and validates the variant identifier at its original child path.
func (scanner *bodyScanner) jobTag(state *jobBodyState, childPath string) error {
	value, err := scanner.typed(stringBody, childPath, state.depth+1)
	if err != nil {
		failure := err.(*bodyFailure)
		failure.detail = strings.Replace(failure.detail, "expected a string", "expected variant identifier", 1)
		return err
	}
	state.name = value.(string)
	if state.name != "index" && state.name != "notify" && state.name != "push" {
		failure := scanner.dataFailure("unknown variant `" + state.name + "`, expected one of `index`, `notify`, `push`").(*bodyFailure)
		failure.path = childPath
		return failure
	}
	return nil
}

// decodeJobVariant replays buffered payload fields and delays their failures to the outer position.
func decodeJobVariant(state jobBodyState) (any, error) {
	kind := structBody("ScheduledDeliveryJob", defaultBodyField("database", optionalBody(stringBody), nil), defaultBodyField("max_candidates", optionalBody(bodyType{kind: "unsigned", name: "usize"}), nil))
	if state.name == "index" {
		kind = structBody("ScheduledIndexJob", defaultBodyField("metadata_file", optionalBody(stringBody), nil), defaultBodyField("notify", boolBody, false), defaultBodyField("push", boolBody, false))
	}
	kind.deniesUnknown = true
	delayed := func(message string) (any, error) {
		return nil, &bodyFailure{detail: message, isData: true, path: state.path, needsPosition: true}
	}
	encoded := "{" + strings.Join(state.parts, ",") + "}"
	if state.isSequence {
		encoded = "[" + strings.Join(state.parts, ",") + "]"
	}
	buffer := bodyScanner{body: []byte(encoded)}
	value, err := buffer.typed(kind, "", 0)
	if err != nil {
		failure := err.(*bodyFailure)
		message := failure.detail
		if index := strings.LastIndex(message, " at line "); index >= 0 {
			message = message[:index]
		}
		if state.isSequence && !failure.isData && len(state.parts) > len(kind.fields) {
			message = fmt.Sprintf("invalid length %d, expected %d elements in sequence", len(state.parts), len(kind.fields))
		}
		return delayed(message)
	}
	return projectJob(state.name, value.(map[string]any)), nil
}

// projectJob retains optional pointers and variant defaults in the scheduler job value.
func projectJob(name string, fields map[string]any) scheduler.Job {
	job := scheduler.Job{Kind: name}
	if value, ok := fields["metadata_file"].(string); ok {
		job.MetadataFile = &value
	}
	if value, ok := fields["notify"].(bool); ok {
		job.Notify = value
	}
	if value, ok := fields["push"].(bool); ok {
		job.Push = value
	}
	if value, ok := fields["database"].(string); ok {
		job.Database = &value
	}
	if value, ok := fields["max_candidates"].(uint64); ok {
		job.MaxCandidates = &value
	}
	return job
}
