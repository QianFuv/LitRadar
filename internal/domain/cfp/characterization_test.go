package cfp

import (
	"encoding/json"
	"reflect"
	"testing"
	"time"
)

func testPointer[T any](value T) *T { return &value }

// TestStructDecodingPreservesObjectAndSequenceContracts fixes the persisted field rules.
func TestStructDecodingPreservesObjectAndSequenceContracts(t *testing.T) {
	type payload struct {
		Name     string   `json:"name"`
		Optional *string  `json:"optional"`
		Items    []string `json:"items" default:"true"`
	}
	cases := []struct {
		name          string
		input         string
		deniesUnknown bool
		isValid       bool
		want          payload
	}{
		{"object defaults", `{"name":"entry"}`, true, true, payload{Name: "entry", Items: []string{}}},
		{"nullable pointer", `{"name":"entry","optional":null}`, true, true, payload{Name: "entry", Items: []string{}}},
		{"pointer and slice", `{"name":"entry","optional":"value","items":["one"]}`, true, true, payload{"entry", testPointer("value"), []string{"one"}}},
		{"positional defaults", `["entry",null]`, true, true, payload{Name: "entry", Items: []string{}}},
		{"positional nullable still required", `["entry"]`, true, false, payload{}},
		{"too many positions", `["entry",null,[],false]`, true, false, payload{}},
		{"missing required", `{}`, true, false, payload{}},
		{"null scalar", `{"name":null}`, true, false, payload{}},
		{"null slice", `{"name":"entry","items":null}`, true, false, payload{}},
		{"null slice item", `{"name":"entry","items":[null]}`, true, false, payload{}},
		{"duplicate known", `{"name":"entry","name":"entry"}`, false, false, payload{}},
		{"unknown rejected", `{"name":"entry","extra":1}`, true, false, payload{}},
		{"duplicate unknown ignored", `{"name":"entry","extra":1,"extra":2}`, false, true, payload{Name: "entry", Items: []string{}}},
		{"unknown invalid surrogate ignored", `{"name":"entry","extra":"\ud800"}`, false, true, payload{Name: "entry", Items: []string{}}},
		{"invalid utf8", "{\"name\":\"\xff\"}", true, false, payload{}},
		{"invalid surrogate", `{"name":"\ud800"}`, true, false, payload{}},
		{"trailing document", `{"name":"entry"}{}`, true, false, payload{}},
		{"scalar document", `true`, true, false, payload{}},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			var actual payload
			err := decodeStruct([]byte(test.input), &actual, test.deniesUnknown)
			if (err == nil) != test.isValid {
				t.Fatalf("valid=%v error=%v", test.isValid, err)
			}
			if test.isValid && !reflect.DeepEqual(actual, test.want) {
				t.Fatalf("decoded=%#v want=%#v", actual, test.want)
			}
		})
	}
}

// TestSourceUnmarshalDoesNotReplaceValueOnFailure protects the wire wrapper's atomic assignment.
func TestSourceUnmarshalDoesNotReplaceValueOnFailure(t *testing.T) {
	source := sourceFixture()
	want := source
	if err := json.Unmarshal([]byte(`{"catalogIds":[],"journalTitle":"changed"}`), &source); err == nil {
		t.Fatal("incomplete source accepted")
	}
	if !reflect.DeepEqual(source, want) {
		t.Fatal("invalid payload changed the source")
	}
}

func sourceFixture() Source {
	return Source{CatalogIds: []string{"journal"}, JournalTitle: "Journal", Title: "  Sample\tcall  ", TypeText: "special issue", SourceUrl: "https://example.org/call", CheckedOn: "2026-01-01"}
}

// TestSourceAdmissionPreservesArchivedAmbiguity fixes validation and archive exceptions.
func TestSourceAdmissionPreservesArchivedAmbiguity(t *testing.T) {
	cases := []struct {
		name    string
		change  func(*Source)
		isValid bool
	}{
		{"undated source", func(source *Source) {}, true},
		{"unknown type", func(source *Source) { source.TypeText = "other" }, false},
		{"editorial entry", func(source *Source) { source.EntryStage = testPointer(Revision) }, false},
		{"credentials", func(source *Source) { source.SourceUrl = "https://user@example.org/call" }, false},
		{"empty catalog", func(source *Source) { source.CatalogIds = []string{" "} }, false},
		{"no catalogs", func(source *Source) { source.CatalogIds = nil }, false},
		{"blocked title", func(source *Source) { source.Title = "Access denied" }, false},
		{"bad timezone", func(source *Source) { source.TimeZone = testPointer("Not/AZone") }, false},
		{"invalid checked date", func(source *Source) { source.CheckedOn = "2026-02-30" }, false},
		{"non padded date", func(source *Source) { source.CheckedOn = "2026-1-01" }, false},
		{"active conflicting dates", func(source *Source) { source.DateText = "Deadline 2026-02-30" }, false},
		{"historical conflicting dates", func(source *Source) { source.DateText = "Deadline 2026-02-30"; source.IsHistorical = true }, true},
		{"closed conflicting dates", func(source *Source) {
			source.DateText = "Deadline 2026-02-30"
			source.StatusText = testPointer("closed")
		}, true},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			source := sourceFixture()
			test.change(&source)
			notice := ParseSource(source)
			if (notice != nil) != test.isValid {
				t.Fatalf("notice=%#v valid=%v", notice, test.isValid)
			}
			if notice != nil && notice.Dates == nil {
				t.Fatal("admitted source has nil dates")
			}
		})
	}
}

// TestSourceNormalizationPreservesPriorityAndContent fixes mixed classification precedence.
func TestSourceNormalizationPreservesPriorityAndContent(t *testing.T) {
	source := sourceFixture()
	source.TypeText = "proposal conference special general"
	source.Scope = " a\t b "
	source.Requirements = " first\r\nsecond "
	source.StatusText = testPointer("closed invitation only")
	source.RawDateText = " unresolved\tdate "
	want := &Notice{Id: "https://example.org/call#Sample call", Title: "Sample call", Scope: "a b", Requirements: "first\nsecond", Kind: ProposalKind, SourceUrl: source.SourceUrl, CheckedOn: source.CheckedOn, Dates: []Date{}, EntryStage: Proposal, SourceStatus: testPointer(Closed), RawDateText: "unresolved date"}
	if actual := ParseSource(source); !reflect.DeepEqual(actual, want) {
		t.Fatalf("notice=%#v want=%#v", actual, want)
	}
	source.TypeText, source.Title, source.StatusText = "general", "2026年度选题", nil
	if actual := ParseSource(source); actual == nil || actual.TopicYear == nil || *actual.TopicYear != 2026 {
		t.Fatalf("annual topic year lost: %#v", actual)
	}
}

// TestEntryDeadlinePreservesMandatoryGateAndTiePrecedence fixes admission ordering.
func TestEntryDeadlinePreservesMandatoryGateAndTiePrecedence(t *testing.T) {
	gate := Date{Date: "2026-03-01", Stage: Abstract, IsExclusive: true}
	cases := []struct {
		name  string
		dates []Date
		want  *Date
	}{
		{"gate before paper priority", []Date{{Date: "2026-01-01", Stage: Paper}, gate}, &gate},
		{"exclusive wins equal gate", []Date{{Date: gate.Date, Stage: Proposal}, gate}, &gate},
		{"optional gate ignored", []Date{{Date: "2026-01-01", Stage: Abstract, IsOptional: true}, gate}, &gate},
		{"first paper retained", []Date{{Date: "2026-04-01", Stage: Paper}, {Date: "2026-02-01", Stage: Paper}}, &Date{Date: "2026-04-01", Stage: Paper}},
		{"no required gate", []Date{{Stage: Paper, IsOptional: true}}, nil},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			if actual := (Notice{Dates: test.dates, EntryStage: Paper}).EntryDeadline(); !reflect.DeepEqual(actual, test.want) {
				t.Fatalf("deadline=%#v want=%#v", actual, test.want)
			}
		})
	}
}

// TestNoticeStatePreservesTimezoneAndSourcePrecedence fixes availability at a single instant.
func TestNoticeStatePreservesTimezoneAndSourcePrecedence(t *testing.T) {
	now := time.Date(2026, 1, 10, 12, 0, 0, 0, time.UTC)
	deadline := []Date{{Date: "2026-01-10", Stage: Paper}}
	opening := []Date{{Date: "2026-01-11", Stage: Opens}}
	cases := []struct {
		name   string
		notice Notice
		want   State
	}{
		{"closed overrides history", Notice{SourceStatus: testPointer(Closed), IsHistorical: true, RawDateText: "unresolved"}, Closed},
		{"raw historical", Notice{IsHistorical: true, RawDateText: "unresolved"}, Historical},
		{"raw invitation", Notice{SourceStatus: testPointer(InvitationOnly), RawDateText: "unresolved"}, InvitationOnly},
		{"raw unknown", Notice{RawDateText: "unresolved"}, Uncertain},
		{"unknown zone deadline", Notice{Dates: deadline, EntryStage: Paper}, Uncertain},
		{"unknown zone empty persisted status", Notice{Dates: deadline, EntryStage: Paper, SourceStatus: testPointer(State(""))}, State("")},
		{"unknown deadline historical", Notice{Dates: deadline, EntryStage: Paper, IsHistorical: true}, Historical},
		{"unknown opening historical", Notice{Dates: opening, IsHistorical: true}, Uncertain},
		{"known zone inclusive", Notice{Dates: deadline, EntryStage: Paper, TimeZone: testPointer("UTC")}, Open},
		{"invalid persisted zone", Notice{Dates: deadline, EntryStage: Paper, TimeZone: testPointer("Not/AZone")}, Open},
		{"exclusive closes today", Notice{Dates: []Date{{Date: "2026-01-10", Stage: Paper, IsExclusive: true}}, EntryStage: Paper, TimeZone: testPointer("UTC")}, Closed},
		{"upcoming", Notice{Dates: opening, TimeZone: testPointer("UTC")}, Upcoming},
		{"invitation before opening", Notice{Dates: opening, TimeZone: testPointer("UTC"), SourceStatus: testPointer(InvitationOnly)}, InvitationOnly},
		{"expired topic year", Notice{TopicYear: testPointer(int32(2025)), IsHistorical: true}, Closed},
		{"undated", Notice{}, Undated},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			if actual := test.notice.State(now); actual != test.want {
				t.Fatalf("state=%s want=%s", actual, test.want)
			}
		})
	}
}

// TestDateClausesPreserveAmbiguityAndMetadata fixes admission and conflict rules.
func TestDateClausesPreserveAmbiguityAndMetadata(t *testing.T) {
	cases := []struct {
		name  string
		input string
		want  []Date
	}{
		{"no date", "Welcome", []Date{}},
		{"unresolved paper", "Deadline TBD", []Date{}},
		{"numeric unresolved gate", "Deadline 03/04/2026", nil},
		{"invalid required calendar", "Deadline 2026-02-30", nil},
		{"invalid editorial calendar", "Decision 2026-02-30", []Date{}},
		{"compatibility digits normalized", "Deadline ２０２６-０３-０４", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "Deadline ２０２６-０３-０４"}}},
		{"non ASCII digits rejected", "Deadline ٢٠٢٦-٠٣-٠٤", nil},
		{"english month", "Deadline 4th March 2026", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "Deadline 4th March 2026"}}},
		{"unambiguous numeric", "Deadline 31/03/2026", []Date{{Date: "2026-03-31", Stage: Paper, OriginalText: "Deadline 31/03/2026"}}},
		{"exclusive English", "Submit before March 4, 2026", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "Submit before March 4, 2026", IsExclusive: true}}},
		{"exclusive Chinese", "投稿截止2026年3月4日前", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "投稿截止2026年3月4日前", IsExclusive: true}}},
		{"previous clause", "Abstract deadline;2026-03-04", []Date{{Date: "2026-03-04", Stage: Abstract, OriginalText: "2026-03-04"}}},
		{"mandatory unresolved abstract", "Abstract deadline TBD;Paper deadline 2026-03-04", nil},
		{"optional unresolved abstract", "Optional abstract deadline TBD;Paper deadline 2026-03-04", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "Paper deadline 2026-03-04"}}},
		{"two required dates", "Deadline 2026-03-04;Deadline 2026-04-04", nil},
		{"one extension removes optional", "Deadline 2026-03-04;Optional deadline 2026-05-04;Deadline extended 2026-04-04", []Date{{Date: "2026-04-04", Stage: Paper, OriginalText: "Deadline extended 2026-04-04"}}},
		{"two extensions", "Deadline extended 2026-03-04;Deadline extended 2026-04-04", nil},
		{"last duplicate metadata", "Deadline 2026-03-04;Optional deadline 2026-03-04", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "Optional deadline 2026-03-04", IsOptional: true}}},
		{"ambiguous and", "Deadline March 4 and 5, 2026", nil},
		{"invalid recognized window", "Submission window February 30-31, 2026", nil},
		{"cross year window", "Submission window December 20-January 5, 2026", []Date{{Date: "2025-12-20", Stage: Opens, OriginalText: "Submission window December 20-January 5, 2026"}, {Date: "2026-01-05", Stage: Paper, OriginalText: "Submission window December 20-January 5, 2026"}}},
		{"day first window", "Submission window 4 March-5 April 2026", []Date{{Date: "2026-03-04", Stage: Opens, OriginalText: "Submission window 4 March-5 April 2026"}, {Date: "2026-04-05", Stage: Paper, OriginalText: "Submission window 4 March-5 April 2026"}}},
		{"shared month window", "Submission window 4-5 March 2026", []Date{{Date: "2026-03-04", Stage: Opens, OriginalText: "Submission window 4-5 March 2026"}, {Date: "2026-03-05", Stage: Paper, OriginalText: "Submission window 4-5 March 2026"}}},
		{"sentence split", "Deadline 2026-03-04. Decision 2026-04-04", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "Deadline 2026-03-04."}, {Date: "2026-04-04", Stage: Decision, OriginalText: "Decision 2026-04-04"}}},
		{"date before stage context", "2026-03-04 submission deadline", []Date{{Date: "2026-03-04", Stage: Paper, OriginalText: "2026-03-04 submission deadline"}}},
		{"explicit window", "Submission window 2026-03-04 to 2026-04-04", []Date{{Date: "2026-03-04", Stage: Opens, OriginalText: "Submission window 2026-03-04 to 2026-04-04"}, {Date: "2026-04-04", Stage: Paper, OriginalText: "Submission window 2026-03-04 to 2026-04-04"}}},
		{"opening after deadline", "Submissions open: 2026-05-04;Deadline 2026-04-04", nil},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			if actual := ParseDates(test.input, Paper); !reflect.DeepEqual(actual, test.want) {
				t.Fatalf("dates=%#v want=%#v", actual, test.want)
			}
		})
	}
}

// TestDefaultSliceDecodingDoesNotClearOmittedPointers fixes decoding into an existing target.
func TestDefaultSliceDecodingDoesNotClearOmittedPointers(t *testing.T) {
	type payload struct {
		Optional *string  `json:"optional"`
		Items    []string `json:"items" default:"true"`
	}
	actual := payload{testPointer("retained"), []string{"old"}}
	if err := decodeStruct([]byte(`{}`), &actual, true); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(actual, payload{testPointer("retained"), []string{}}) {
		t.Fatalf("decoded=%#v", actual)
	}
}

// TestPatternCapturesPreservesUnicodeAndCaptureOffsets fixes the extraction regex contract.
func TestPatternCapturesPreservesUnicodeAndCaptureOffsets(t *testing.T) {
	cases := []struct {
		pattern string
		input   string
		want    [][]int
	}{
		{`\b(\w+)\b`, "汉字", [][]int{{0, 6, 0, 6}}},
		{`(\d+)\s([\w]+)`, "١\u00a0汉", [][]int{{0, 7, 0, 2, 4, 7}}},
		{`\b(cat)\b`, "écat cat猫 cat", [][]int{{13, 16, 13, 16}}},
		{`(x)?\b(cat)\b`, "cat", [][]int{{0, 3, -1, -1, 0, 3}}},
		{`\\b`, `\b`, [][]int{{0, 2}}},
	}
	for _, test := range cases {
		t.Run(test.pattern+test.input, func(t *testing.T) {
			if actual := PatternCaptures(test.pattern, test.input); !reflect.DeepEqual(actual, test.want) {
				t.Fatalf("captures=%v want=%v", actual, test.want)
			}
		})
	}
}
