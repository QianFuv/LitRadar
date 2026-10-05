package logfilter

import (
	"errors"
	"math"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

const (
	Off = iota
	Error
	Warn
	Info
	Debug
	Trace
)

// DebugValue contains the original typed value's debug representation.
type DebugValue string

type valueMatch struct {
	kind       byte
	value      any
	expression *fieldRegex
}
type directive struct {
	target, span *string
	field        string
	hasField     bool
	value        *valueMatch
	level        int
}

// Filter separates static callsite rules from scope-dependent span rules.
type Filter struct {
	statics, dynamics []directive
	maximum           int
}

// Parse validates and compiles persisted EnvFilter syntax without consulting environment variables.
func Parse(value string) (*Filter, error) {
	if _, err := settings.Normalize("log_filter", value); err != nil {
		return nil, errors.New("invalid LitRadar log filter")
	}
	filter := &Filter{}
	for _, text := range strings.Split(value, ",") {
		if text == "" {
			continue
		}
		current, err := parseDirective(text)
		if err != nil {
			return nil, errors.New("invalid LitRadar log filter")
		}
		filter.maximum = max(filter.maximum, current.level)
		if current.span != nil || current.hasField {
			filter.dynamics = replaceDirective(filter.dynamics, current)
		}
		if current.span == nil && current.value == nil {
			filter.statics = replaceDirective(filter.statics, current)
		}
		if current.value != nil {
			filter.maximum = Trace
		}
	}
	slices.SortFunc(filter.statics, func(left, right directive) int {
		first, second := -1, -1
		if left.target != nil {
			first = len(*left.target)
		}
		if right.target != nil {
			second = len(*right.target)
		}
		if first != second {
			return second - first
		}
		if left.hasField != right.hasField {
			if left.hasField {
				return -1
			}
			return 1
		}
		return strings.Compare(right.field, left.field)
	})
	return filter, nil
}

func sameText(left, right *string) bool {
	return left == nil && right == nil || left != nil && right != nil && *left == *right
}
func replaceDirective(values []directive, current directive) []directive {
	for position, previous := range values {
		if sameText(previous.target, current.target) && sameText(previous.span, current.span) && previous.hasField == current.hasField && previous.field == current.field && sameValue(previous.value, current.value) {
			values[position] = current
			return values
		}
	}
	return append(values, current)
}
func sameValue(left, right *valueMatch) bool {
	if left == nil || right == nil {
		return left == right
	}
	return left.kind == right.kind && (left.value == right.value || left.kind == 'n')
}

func parseLevel(value string) (int, bool) {
	for level, name := range []string{"off", "error", "warn", "info", "debug", "trace"} {
		if strings.EqualFold(value, name) {
			return level, true
		}
	}
	number, err := strconv.ParseUint(strings.TrimPrefix(value, "+"), 10, 64)
	return int(number), err == nil && number <= 5
}

func parseDirective(value string) (directive, error) {
	const (
		start = iota
		levelOrTarget
		span
		field
		fields
		target
		level
	)
	state, offset := start, 0
	result := directive{level: Trace}
	textPointer := func(text string) *string { return &text }
	for position, character := range strings.TrimSpace(value) {
		switch state {
		case start:
			if character == '[' {
				state, offset = span, position+1
			} else {
				state, offset = levelOrTarget, position
			}
		case levelOrTarget:
			if character == '=' {
				result.target = textPointer(value[offset:position])
				state, offset = level, position+1
			} else if character == '[' {
				result.target = textPointer(value[offset:position])
				state, offset = span, position+1
			}
		case span:
			if character == ']' {
				result.span = textPointer(value[offset:position])
				state = target
			} else if character == '{' {
				if position > offset {
					result.span = textPointer(value[offset:position])
				}
				state, offset = field, position+1
			}
		case field:
			if character == '}' {
				parts := strings.Split(value[offset:position], "=")
				result.hasField = true
				result.field = parts[0]
				if len(parts) > 1 {
					parsed, err := parseValue(parts[1])
					if err != nil {
						return result, err
					}
					result.value = parsed
				}
				state = fields
			}
		case fields:
			state = target
		case target:
			state, offset = level, position+1
		}
	}
	if state == levelOrTarget {
		if parsed, exists := parseLevel(value[offset:]); exists {
			result.level = parsed
		} else {
			result.target = textPointer(value[offset:])
		}
	} else if state == level && value[offset:] != "" {
		result.level, _ = parseLevel(value[offset:])
	}
	return result, nil
}

var floatSyntax = regexp.MustCompile(`^[+-]?(?:(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?|(?i:inf(?:inity)?|nan))$`)

func parseValue(value string) (*valueMatch, error) {
	if value == "true" || value == "false" {
		return &valueMatch{kind: 'b', value: value == "true"}, nil
	}
	if number, err := strconv.ParseUint(strings.TrimPrefix(value, "+"), 10, 64); err == nil {
		return &valueMatch{kind: 'u', value: number}, nil
	}
	if number, err := strconv.ParseInt(value, 10, 64); err == nil {
		return &valueMatch{kind: 'i', value: number}, nil
	}
	numeric := value
	if strings.EqualFold(strings.TrimLeft(value, "+-"), "infinity") {
		numeric = strings.ReplaceAll(strings.ToLower(value), "infinity", "inf")
	}
	if floatSyntax.MatchString(value) && strings.EqualFold(strings.TrimLeft(value, "+-"), "nan") {
		return &valueMatch{kind: 'n'}, nil
	}
	if floatSyntax.MatchString(value) {
		if number, err := strconv.ParseFloat(numeric, 64); err == nil || errors.Is(err, strconv.ErrRange) {
			return &valueMatch{kind: 'f', value: number}, nil
		}
	}
	expression, err := compileFieldRegex(value)
	return &valueMatch{kind: 'r', value: value, expression: expression}, err
}

func (pattern *valueMatch) matches(value any) bool {
	switch actual := value.(type) {
	case bool:
		return pattern.kind == 'b' && actual == pattern.value
	case int:
		return pattern.matches(int64(actual))
	case uint:
		return pattern.matches(uint64(actual))
	case int32:
		return pattern.matches(int64(actual))
	case uint32:
		return pattern.matches(uint64(actual))
	case int64:
		return pattern.kind == 'i' && actual == pattern.value || pattern.kind == 'u' && actual >= 0 && uint64(actual) == pattern.value
	case uint64:
		return pattern.kind == 'u' && actual == pattern.value
	case float64:
		return pattern.kind == 'n' && math.IsNaN(actual) || pattern.kind == 'f' && math.Abs(actual-pattern.value.(float64)) < 0x1p-52
	case string:
		return pattern.kind == 'r' && pattern.expression.matches(actual)
	case DebugValue:
		return pattern.kind == 'r' && pattern.expression.matches(string(actual))
	}
	return false
}

func (rule directive) cares(target, name string, values map[string]any, isSpan bool) bool {
	if rule.target != nil && !strings.HasPrefix(target, *rule.target) {
		return false
	}
	if rule.span != nil && *rule.span != name {
		return false
	}
	if rule.hasField {
		_, exists := values[rule.field]
		return exists || !isSpan && name == ""
	}
	return true
}

func (filter *Filter) staticLevel(target string, values map[string]any, isSpan bool) int {
	for _, rule := range filter.statics {
		if rule.target != nil && !strings.HasPrefix(target, *rule.target) {
			continue
		}
		if rule.hasField && !isSpan {
			if _, exists := values[rule.field]; !exists {
				continue
			}
		}
		return rule.level
	}
	return Off
}

// Entry freezes a span's matching level at entry time while retaining its recorded fields.
type Entry struct {
	Span  *Span
	Level int
}
type spanRule struct {
	directive
	didMatch bool
}

// Span retains sticky typed matches across records and independent scope entries.
type Span struct {
	mutex  sync.Mutex
	name   string
	values map[string]any
	rules  []spanRule
	base   int
}

// NewSpan applies callsite admission before recording fields; nil represents a disabled span.
func (filter *Filter) NewSpan(target, name string, level int, values map[string]any, scope []Entry) *Span {
	if level > filter.maximum {
		return nil
	}
	span := &Span{name: name, values: map[string]any{}}
	for _, rule := range filter.dynamics {
		if rule.cares(target, name, values, true) {
			if rule.value == nil {
				span.base = max(span.base, rule.level)
			} else {
				span.rules = append(span.rules, spanRule{directive: rule})
			}
		}
	}
	if len(span.rules) == 0 && span.base == Off {
		hasMatch := false
		for _, rule := range filter.dynamics {
			if rule.cares(target, name, values, true) {
				hasMatch = true
				break
			}
		}
		if !hasMatch && !filter.enabled(target, level, values, scope, true) {
			return nil
		}
	}
	for name, value := range values {
		span.values[name] = nil
		span.record(name, value)
	}
	return span
}

func (span *Span) record(name string, value any) {
	span.values[name] = value
	for position := range span.rules {
		rule := &span.rules[position]
		if rule.field == name && rule.value.matches(value) {
			rule.didMatch = true
		}
	}
}

// Record updates declared fields without altering an already entered scope's level.
func (span *Span) Record(name string, value any) {
	if span == nil {
		return
	}
	span.mutex.Lock()
	defer span.mutex.Unlock()
	if _, exists := span.values[name]; exists {
		span.record(name, value)
	}
}

// Enter snapshots the current match level, preserving original sticky record semantics.
func (span *Span) Enter() Entry {
	if span == nil {
		return Entry{}
	}
	span.mutex.Lock()
	defer span.mutex.Unlock()
	level := span.base
	for _, rule := range span.rules {
		if rule.didMatch {
			level = max(level, rule.level)
		}
	}
	return Entry{Span: span, Level: level}
}

// Fields returns an independent rendering snapshot, excluding unrecorded fields.
func (span *Span) Fields() map[string]any {
	span.mutex.Lock()
	defer span.mutex.Unlock()
	result := map[string]any{"name": span.name}
	for name, value := range span.values {
		if value != nil {
			result[name] = value
		}
	}
	return result
}

// Enabled evaluates an event against its original target, declared fields and active scope.
func (filter *Filter) Enabled(target string, level int, values map[string]any, scope []Entry) bool {
	return filter.enabled(target, level, values, scope, false)
}
func (filter *Filter) enabled(target string, level int, values map[string]any, scope []Entry, isSpan bool) bool {
	if level <= Off || level > filter.maximum {
		return false
	}
	for _, entry := range scope {
		if entry.Span != nil && entry.Level >= level {
			return true
		}
	}
	return filter.staticLevel(target, values, isSpan) >= level
}
