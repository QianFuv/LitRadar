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
		scanner.position++
		if depth+1 >= 128 {
			return nil, scanner.failure("recursion limit exceeded", false)
		}
		end, collection := byte('}'), "an object"
		if token == '[' {
			end, collection = ']', "a list"
		}
		count := 0
		for scanner.peek() != end {
			if scanner.position == len(scanner.body) {
				return nil, scanner.failure("EOF while parsing "+collection, false)
			}
			if count > 0 {
				if err := scanner.separator(end); err != nil {
					return nil, err
				}
			}
			childPath := fmt.Sprintf("%s[%d]", path, count)
			if token == '{' {
				if scanner.peek() != '"' {
					return nil, scanner.failure("key must be a string", true)
				}
				keyStart := scanner.position
				if err := scanner.stringValue(); err != nil {
					return nil, err
				}
				var key string
				_ = json.Unmarshal(scanner.body[keyStart:scanner.position], &key)
				childPath = path + "." + key
				if scanner.peek() != ':' {
					return nil, scanner.failure("expected `:`", true)
				}
				scanner.position++
			}
			if _, err := scanner.content(childPath, depth+1); err != nil {
				return nil, err
			}
			count++
		}
		scanner.position++
	case '"':
		err = scanner.stringValue()
	case 'n':
		err = scanner.literal("null")
	case 't':
		err = scanner.literal("true")
	case 'f':
		err = scanner.literal("false")
	default:
		if token != '-' && (token < '0' || token > '9') {
			return nil, scanner.failure("expected value", true)
		}
		err = scanner.number()
		if err == nil {
			if _, numberErr := sources.ParseNumber(json.Number(scanner.body[start:scanner.position])); numberErr != nil {
				err = scanner.failure("number out of range", false)
			}
		}
	}
	if err != nil {
		return nil, err
	}
	return scanner.body[start:scanner.position], nil
}

func (scanner *bodyScanner) job(path string, depth int) (any, error) {
	token := scanner.peek()
	if token != '{' && token != '[' {
		_, err := scanner.typed(structBody("ScheduledJobSpec"), path, depth)
		if err != nil {
			failure := err.(*bodyFailure)
			failure.detail = strings.Replace(failure.detail, "struct ScheduledJobSpec", "internally tagged enum ScheduledJobSpec", 1)
		}
		return nil, err
	}
	scanner.position++
	if depth+1 >= 128 {
		return nil, scanner.failure("recursion limit exceeded", false)
	}
	isSequence := token == '['
	end := byte('}')
	if isSequence {
		end = ']'
	}
	name := ""
	parts := []string{}
	count := 0
	for scanner.peek() != end {
		if scanner.position == len(scanner.body) {
			collection := "an object"
			if isSequence {
				collection = "a list"
			}
			return nil, scanner.failure("EOF while parsing "+collection, false)
		}
		if count > 0 {
			if err := scanner.separator(end); err != nil {
				return nil, err
			}
		}
		key := ""
		encodedKey := ""
		childPath := fmt.Sprintf("%s[%d]", path, count)
		if !isSequence {
			if scanner.peek() != '"' {
				return nil, scanner.failure("key must be a string", true)
			}
			start := scanner.position
			if err := scanner.stringValue(); err != nil {
				return nil, err
			}
			encodedKey = string(scanner.body[start:scanner.position])
			_ = json.Unmarshal([]byte(encodedKey), &key)
			childPath = path + "." + key
			if key == "kind" && name != "" {
				scanner.peek()
				if scanner.peek() == '}' {
					scanner.position++
				}
				return nil, scanner.dataFailure("duplicate field `kind`")
			}
			if scanner.peek() != ':' {
				return nil, scanner.failure("expected `:`", true)
			}
			scanner.position++
		}
		if key == "kind" || isSequence && count == 0 {
			value, err := scanner.typed(stringBody, childPath, depth+1)
			if err != nil {
				failure := err.(*bodyFailure)
				failure.detail = strings.Replace(failure.detail, "expected a string", "expected variant identifier", 1)
				return nil, err
			}
			name = value.(string)
			if name != "index" && name != "notify" && name != "push" {
				failure := scanner.dataFailure("unknown variant `" + name + "`, expected one of `index`, `notify`, `push`").(*bodyFailure)
				failure.path = childPath
				return nil, failure
			}
		} else {
			raw, err := scanner.content(childPath, depth+1)
			if err != nil {
				return nil, err
			}
			if isSequence {
				parts = append(parts, string(raw))
			} else {
				parts = append(parts, encodedKey+":"+string(raw))
			}
		}
		count++
	}
	scanner.position++
	if name == "" {
		return nil, scanner.dataFailure("missing field `kind`")
	}
	kind := structBody("ScheduledDeliveryJob", defaultBodyField("database", optionalBody(stringBody), nil), defaultBodyField("max_candidates", optionalBody(bodyType{kind: "unsigned", name: "usize"}), nil))
	if name == "index" {
		kind = structBody("ScheduledIndexJob", defaultBodyField("metadata_file", optionalBody(stringBody), nil), defaultBodyField("notify", boolBody, false), defaultBodyField("push", boolBody, false))
	}
	kind.deniesUnknown = true
	delayed := func(message string) (any, error) {
		return nil, &bodyFailure{detail: message, isData: true, path: path, needsPosition: true}
	}
	encoded := "{" + strings.Join(parts, ",") + "}"
	if isSequence {
		encoded = "[" + strings.Join(parts, ",") + "]"
	}
	buffer := bodyScanner{body: []byte(encoded)}
	value, err := buffer.typed(kind, "", 0)
	if err != nil {
		failure := err.(*bodyFailure)
		message := failure.detail
		if index := strings.LastIndex(message, " at line "); index >= 0 {
			message = message[:index]
		}
		if isSequence && !failure.isData && len(parts) > len(kind.fields) {
			message = fmt.Sprintf("invalid length %d, expected %d elements in sequence", len(parts), len(kind.fields))
		}
		return delayed(message)
	}
	fields := value.(map[string]any)
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
	return job, nil
}
