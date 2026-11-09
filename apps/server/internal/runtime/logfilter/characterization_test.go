package logfilter

import (
	"math"
	"reflect"
	"sync"
	"testing"
)

// TestFieldRegexPreservesOrderedEndOfInputMatching fixes the runtime regex contract.
func TestFieldRegexPreservesOrderedEndOfInputMatching(t *testing.T) {
	cases := []struct {
		pattern string
		input   string
		want    bool
	}{
		{"a", "a", true}, {"a", "ba", true}, {"a", "ab", false},
		{"a|ab", "ab", false}, {"ab|a", "ab", true},
		{"a*", "aaa", true}, {"a*?", "aaa", false}, {"a+", "aaa", true},
		{"(?U)a*", "aaa", false}, {"(?U)a*?", "aaa", true},
		{"(?i:abc)", "ABC", true}, {"(?i:a)b", "AB", false},
		{"(?i)[a--A]", "a", false}, {"(?i)[a&&A]", "A", true},
		{"[a-z&&[^b]]+", "abc", false}, {"[a-z--b]+", "ac", true},
		{"[a~~b]+", "ab", true}, {"[[:digit:]]+", "123", true},
		{"[[:digit:]]+", "١٢٣", false}, {`\d+`, "١٢٣", true},
		{`(?-u:\d+)`, "١٢٣", false}, {`(?-u:\bword\b)`, "word", true},
		{`(?-u:\<word\>)`, "word", true}, {`\pL+`, "汉字", true},
		{`(?i:k)`, "K", true}, {`(?i-u:k)`, "K", false},
		{".", "\n", false}, {"(?s:.)", "\n", true},
		{"(?m:^b$)", "a\nb", true}, {"(?mR:^b$)", "a\rb", true},
		{"(?mR:^b$)", "a\r\nb", true}, {"(?R:.)", "\r", false},
		{`\x41\u0042\U00000043`, "ABC", true},
		{`(?x:\x 4 1)`, "A", true}, {"[]a]+", "]aa", true},
		{"(?<name>a)(?P<other>b)", "ab", true}, {"a", "\xff", false},
	}
	for _, test := range cases {
		t.Run(test.pattern+test.input, func(t *testing.T) {
			expression, err := compileFieldRegex(test.pattern)
			if err != nil {
				t.Fatal(err)
			}
			if actual := expression.matches(test.input); actual != test.want {
				t.Fatalf("matched=%v want=%v", actual, test.want)
			}
		})
	}
}

// TestRegexAdmissionPreservesValidatorBoundaries fixes shared explicit syntax rejection cases.
func TestRegexAdmissionPreservesValidatorBoundaries(t *testing.T) {
	for _, pattern := range []string{`\b`, `(?-u:.)`, `(?-u:\D)`, `(?ii:a)`, `(?P<name>a)(?<name>b)`, `\uD800`, `[z-a]`, `a{2}`, `\q`, "\xff"} {
		t.Run(pattern, func(t *testing.T) {
			if _, err := compileFieldRegex(pattern); err == nil {
				t.Fatal("invalid syntax admitted")
			}
		})
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

// TestAsciiClassesPreserveInclusiveBoundaries fixes all POSIX ASCII memberships.
func TestAsciiClassesPreserveInclusiveBoundaries(t *testing.T) {
	cases := []struct {
		name string
		set  runeSet
	}{
		{"alnum", runeSet{'0', '9', 'A', 'Z', 'a', 'z'}},
		{"alpha", runeSet{'A', 'Z', 'a', 'z'}},
		{"ascii", runeSet{0, 127}}, {"blank", runeSet{9, 9, 32, 32}},
		{"cntrl", runeSet{0, 31, 127, 127}}, {"digit", runeSet{'0', '9'}},
		{"graph", runeSet{33, 126}}, {"lower", runeSet{'a', 'z'}},
		{"print", runeSet{32, 126}}, {"punct", runeSet{33, 47, 58, 64, 91, 96, 123, 126}},
		{"space", runeSet{9, 13, 32, 32}}, {"upper", runeSet{'A', 'Z'}},
		{"word", runeSet{'0', '9', 'A', 'Z', '_', '_', 'a', 'z'}},
		{"xdigit", runeSet{'0', '9', 'A', 'F', 'a', 'f'}}, {"unknown", runeSet{}},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			actual := asciiClass(test.name)
			for value := rune(0); value <= 128; value++ {
				if actual.contains(value) != test.set.contains(value) {
					t.Fatalf("class %s differs at %U", test.name, value)
				}
			}
		})
	}
}
