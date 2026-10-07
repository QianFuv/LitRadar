package cnki

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/sources/jfbym"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

// TestCaptchaAcceptedCancellationRetainsId preserves admission completed inside a callback.
func TestCaptchaAcceptedCancellationRetainsId(t *testing.T) {
	session := NewCaptchaSession(5)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	err := session.SolveChallenge(ctx, "url", jfbym.NewFixture(10, 0), func(context.Context, string) (CaptchaPuzzle, error) {
		return testPuzzle(), nil
	}, func(context.Context, CaptchaPuzzle, string) (bool, error) {
		cancel()
		return true, nil
	})
	if err != nil || session.RemainingBudget() != 4 {
		t.Fatalf("accepted cancellation changed quota or result: %v", err)
	}
	attached, err := session.AttachCaptchaId(KnsBase + "/article")
	if err != nil || !strings.HasSuffix(attached, "captchaId=secret-id") {
		t.Fatalf("accepted id lost: %q %v", attached, err)
	}
}

// TestCaptchaFailedFetchAdvancesCandidate preserves the cumulative session candidate offset.
func TestCaptchaFailedFetchAdvancesCandidate(t *testing.T) {
	session := NewCaptchaSession(5)
	wantFailure := errors.New("fetch failed")
	err := session.SolveChallenge(context.Background(), "url", nil, func(context.Context, string) (CaptchaPuzzle, error) {
		return CaptchaPuzzle{}, wantFailure
	}, nil)
	if err != wantFailure {
		t.Fatal(err)
	}
	want, err := jfbym.EncryptPointJson(testPuzzle().SecretKey, 101, 5)
	if err != nil {
		t.Fatal(err)
	}
	err = session.SolveChallenge(context.Background(), "url", jfbym.NewFixture(100.4, 0), func(context.Context, string) (CaptchaPuzzle, error) {
		return testPuzzle(), nil
	}, func(_ context.Context, _ CaptchaPuzzle, point string) (bool, error) {
		if point != want {
			t.Fatalf("prior failed fetch did not advance candidate: %q", point)
		}
		return true, nil
	})
	if err != nil || session.RemainingBudget() != 3 {
		t.Fatalf("%v remaining=%d", err, session.RemainingBudget())
	}
}

// TestCaptchaSuccessMarkerClassification preserves integer grammar and present-null priority.
func TestCaptchaSuccessMarkerClassification(t *testing.T) {
	cases := []struct {
		value any
		want  bool
	}{{true, true}, {"true", true}, {"True", false}, {int(1), true}, {int64(1), true}, {uint64(1), true}, {float64(1), false}, {json.Number("1"), true}, {json.Number("1.0"), false}, {json.Number("1e0"), false}, {json.Number("01"), false}}
	for _, test := range cases {
		if got := successValue(test.value); got != test.want {
			t.Fatalf("marker %#v: %v want %v", test.value, got, test.want)
		}
	}
	if CaptchaCheckSucceeded(map[string]any{"repData": nil, "data": map[string]any{"result": true}}) {
		t.Fatal("present null repData fell back to data")
	}
	if !CaptchaCheckSucceeded(map[string]any{"data": map[string]any{"result": true}}) {
		t.Fatal("absent repData did not fall back to data")
	}
}

// TestAttributeQuotePassPrecedence preserves legacy mixed-quote duplicate handling.
func TestAttributeQuotePassPrecedence(t *testing.T) {
	for _, text := range []string{`<a title='single' title="double">`, `<a title="double" title='single'>`} {
		if got := attrs(text)["title"]; got != "single" {
			t.Fatalf("mixed quote precedence: %q", got)
		}
	}
	if _, exists := attrs(`<a title= "ignored">`)["title"]; exists {
		t.Fatal("attribute with whitespace after equals was admitted")
	}
}

// TestJournalSeenDetailSuppressesWarmup preserves admission before matching or parsing.
func TestJournalSeenDetailSuppressesWarmup(t *testing.T) {
	endpoints := []string{}
	locator := NewJournalLocator([]string{"Journal（A）", "Other"}, nil)
	result, err := resolveJournal(locator, func(endpoint, _ string, _ []scholarly.QueryPair) (string, error) {
		endpoints = append(endpoints, endpoint)
		if endpoint == "journal_search" {
			return `<a href="/knavi/detail?pykm=J">Unmatched</a>`, nil
		}
		return "missing pykm", nil
	})
	if err != nil || result != nil {
		t.Fatalf("%v %v", result, err)
	}
	want := []string{"journal_search", "journal_detail", "journal_search", "journal_search"}
	if !reflect.DeepEqual(endpoints, want) {
		t.Fatalf("seen detail admission changed request order: %v", endpoints)
	}
}

// TestJournalNoCandidatesWarmsOnce preserves the only allowed second search pass.
func TestJournalNoCandidatesWarmsOnce(t *testing.T) {
	endpoints := []string{}
	_, err := resolveJournal(NewJournalLocator([]string{"Journal"}, nil), func(endpoint, _ string, _ []scholarly.QueryPair) (string, error) {
		endpoints = append(endpoints, endpoint)
		if endpoint == "navigation" {
			return "<html></html>", nil
		}
		return "no results", nil
	})
	if err != nil || !reflect.DeepEqual(endpoints, []string{"journal_search", "navigation", "journal_search"}) {
		t.Fatalf("navigation warmup order: %v %v", endpoints, err)
	}
}
