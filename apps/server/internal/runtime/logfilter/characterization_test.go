package logfilter

import (
	"math"
	"reflect"
	"sync"
	"testing"
)

// TestFieldRegexUsesGoSyntax verifies substring matching and the supported dialect.
func TestFieldRegexUsesGoSyntax(t *testing.T) {
	for _, test := range []struct {
		pattern, input string
		want           bool
	}{
		{"a", "ab", true}, {"a$", "ab", false}, {"a|ab", "ab", true},
		{`\d+`, "١٢٣", false}, {`\pL+`, "汉字", true}, {`\bword\b`, "word", true},
	} {
		expression, err := parseValue(test.pattern)
		if err != nil {
			t.Fatal(err)
		}
		if expression.matches(DebugValue(test.input)) != test.want {
			t.Fatal(test)
		}
	}
	for _, pattern := range []string{`(?x:a)`, `(?-u:a)`, `(?R:a)`, `\u0041`, `[z-a]`} {
		if _, err := parseValue(pattern); err == nil {
			t.Fatalf("unsupported syntax admitted: %s", pattern)
		}
	}
}

// TestTypedFieldMatchesPreserveNumericAndDebugRepresentations fixes typed field admission.
func TestTypedFieldMatchesPreserveNumericAndDebugRepresentations(t *testing.T) {
	cases := []struct {
		pattern string
		value   any
		want    bool
	}{
		{"true", true, true}, {"true", "true", false},
		{"1", int(1), true}, {"1", int32(1), true}, {"1", uint32(1), true},
		{"1", uint(1), true}, {"1", int64(-1), false}, {"1", float64(1), false},
		{"-1", int64(-1), true}, {"-1", uint64(1), false},
		{"1.5", float64(1.5), true}, {"1.5", float64(1.5 + 1e-10), false},
		{"nan", math.NaN(), true}, {"nan", math.Inf(1), false}, {"inf", math.Inf(1), false},
		{"abc", DebugValue("abc"), true}, {"abc", []byte("abc"), false},
	}
	for _, test := range cases {
		t.Run(test.pattern, func(t *testing.T) {
			pattern, err := parseValue(test.pattern)
			if err != nil {
				t.Fatal(err)
			}
			if actual := pattern.matches(test.value); actual != test.want {
				t.Fatalf("matched=%v want=%v value=%v", actual, test.want, test.value)
			}
		})
	}
}

// TestSpanMatchesPreserveStickyRecordsAndEntrySnapshots fixes dynamic scope behavior.
func TestSpanMatchesPreserveStickyRecordsAndEntrySnapshots(t *testing.T) {
	filter, err := Parse("off,[task{value=true}]=debug")
	if err != nil {
		t.Fatal(err)
	}
	span := filter.NewSpan("target", "task", Info, map[string]any{"value": false}, nil)
	if span == nil {
		t.Fatal("dynamic span was disabled")
	}
	early := span.Enter()
	span.Record("undeclared", true)
	span.Record("value", true)
	matched := span.Enter()
	span.Record("value", false)
	if early.Level != Off || matched.Level != Debug || span.Enter().Level != Debug {
		t.Fatal("entry snapshot or sticky match lost")
	}
	if filter.Enabled("other", Debug, nil, []Entry{early}) || !filter.Enabled("other", Debug, nil, []Entry{matched}) {
		t.Fatal("scope entry levels changed retroactively")
	}
	fields := span.Fields()
	fields["value"] = "caller change"
	if !reflect.DeepEqual(span.Fields(), map[string]any{"name": "task", "value": false}) {
		t.Fatal("field snapshot aliases span or includes undeclared values")
	}
	if filter.NewSpan("target", "other", Info, nil, nil) != nil {
		t.Fatal("unmatched disabled span admitted")
	}
}

// TestStaticRulesPreservePrefixPriorityAndReplacement fixes deterministic directive selection.
func TestStaticRulesPreservePrefixPriorityAndReplacement(t *testing.T) {
	filter, err := Parse("warn,target=debug,target::specific=error,target=info")
	if err != nil {
		t.Fatal(err)
	}
	if filter.Enabled("target::specific", Info, nil, nil) || !filter.Enabled("target::other", Info, nil, nil) || filter.Enabled("target::other", Debug, nil, nil) || !filter.Enabled("unrelated", Warn, nil, nil) {
		t.Fatal("target priority or last-directive replacement lost")
	}
}

// TestConcurrentSpanRecordsKeepSnapshotsIndependent verifies declared-field synchronization.
func TestConcurrentSpanRecordsKeepSnapshotsIndependent(t *testing.T) {
	filter, err := Parse("off,[task{value=true}]=debug")
	if err != nil {
		t.Fatal(err)
	}
	span := filter.NewSpan("target", "task", Info, map[string]any{"value": false}, nil)
	if span == nil {
		t.Fatal("dynamic span was disabled")
	}
	var workers sync.WaitGroup
	for range 20 {
		workers.Go(func() { span.Record("value", true); span.Enter(); span.Fields() })
	}
	workers.Wait()
	if span.Enter().Level != Debug {
		t.Fatal("concurrent sticky match lost")
	}
}

// TestMatchingOffDirectiveRetainsSpan distinguishes an Off match from no dynamic match.
func TestMatchingOffDirectiveRetainsSpan(t *testing.T) {
	filter, err := Parse("warn,[task]=off")
	if err != nil {
		t.Fatal(err)
	}
	span := filter.NewSpan("target", "task", Warn, nil, nil)
	if span == nil || span.Enter().Level != Off {
		t.Fatal("matching Off directive lost its admitted span")
	}
}
