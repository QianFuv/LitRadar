package cnki

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/sources/jfbym"
)

func TestFrozenCnkiParsers(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/cnki-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Observations []struct {
			Id, Kind, Text, Url, Field string
			Issue, Body                any
			Page                       uint64
			Titles, Issns              []string
			Output                     any
			Fixture                    FixtureData
			Operations                 []map[string]any
			Budget, Failures           uint64
			Distance                   float64
			Attach                     []string
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	if err := decoder.Decode(&fixture); err != nil {
		t.Fatal(err)
	}
	for _, observation := range fixture.Observations {
		t.Run(observation.Id, func(t *testing.T) {
			var value any
			var err error
			switch observation.Kind {
			case "search":
				value, err = ParseJournalSearch(observation.Text)
			case "detail":
				value, err = ParseJournalDetail(observation.Text)
			case "years":
				value, err = ParseYearIssues(observation.Text)
			case "papers":
				value, err = ParseIssueArticles(observation.Text, observation.Issue, observation.Page)
			case "article":
				value, err = ParseArticleDetail(observation.Text, observation.Url)
			case "form":
				value = JournalSearchForm(observation.Text, observation.Field)
			case "challenge":
				var challenge *string
				challenge, err = ExtractChallengeUrl(observation.Text, observation.Url)
				value = map[string]any{"detected": LooksLikeCaptchaChallenge(observation.Text, observation.Url), "url": challenge}
			case "puzzle":
				var puzzle CaptchaPuzzle
				puzzle, err = ParseCaptchaPuzzle(observation.Url, observation.Body)
				value = map[string]any{"challenge_url": puzzle.ChallengeUrl, "captcha_type": puzzle.CaptchaType, "ident": puzzle.Ident, "captcha_id": puzzle.CaptchaId, "return_url": puzzle.ReturnUrl, "secret_key": puzzle.SecretKey, "token": puzzle.Token, "original": puzzle.OriginalImageBase64, "jigsaw": puzzle.JigsawImageBase64, "check": CaptchaCheckRequestBody(puzzle, "ciphertext")}
			case "success":
				value = CaptchaCheckSucceeded(observation.Body)
			case "fixture_decode":
				var decoded FixtureData
				value = json.Unmarshal([]byte(observation.Text), &decoded) == nil
			case "anchor":
				var anchor Anchor
				if json.Unmarshal([]byte(observation.Text), &anchor) == nil {
					encoded, failure := anchor.Encode()
					if failure != nil {
						t.Fatal(failure)
					}
					value = map[string]any{"encoded": encoded}
				}
			case "checkpoint":
				var checkpoint Checkpoint
				if json.Unmarshal([]byte(observation.Text), &checkpoint) == nil {
					encoded, failure := checkpoint.Encode()
					if failure != nil {
						t.Fatal(failure)
					}
					value = map[string]any{"encoded": encoded}
				}
			case "fixture":
				value = fixtureSequence(t, observation.Fixture, observation.Operations)
			case "solve":
				session := NewCaptchaSession(observation.Budget)
				solver := jfbym.NewFixture(observation.Distance, observation.Failures)
				observations := []any{}
				for _, operation := range observation.Operations {
					fetches := 0
					points := []string{}
					accepted := uint64(^uint64(0))
					if raw, ok := operation["accept_after"].(json.Number); ok {
						var parsed uint64
						if json.Unmarshal([]byte(raw), &parsed) != nil {
							t.Fatal(raw)
						}
						accepted = parsed
					}
					failure := session.EnsureAccess(context.Background(), stringField(operation, "text"), stringField(operation, "url"), solver, func(_ context.Context, url string) (CaptchaPuzzle, error) {
						fetches++
						if operation["fail"] == "fetch" {
							return CaptchaPuzzle{}, &Error{Kind: "Request", Message: "fetch failed"}
						}
						return ParseCaptchaPuzzle(url, operation["body"])
					}, func(_ context.Context, puzzle CaptchaPuzzle, point string) (bool, error) {
						points = append(points, point)
						if operation["fail"] == "submit" {
							return false, &Error{Kind: "Request", Message: "submit failed"}
						}
						return uint64(len(points)) >= accepted, nil
					})
					urls := []any{}
					for _, url := range observation.Attach {
						attached, failure := session.AttachCaptchaId(url)
						urls = append(urls, cnkiOutcome(attached, failure))
					}
					observations = append(observations, map[string]any{"result": cnkiOutcome(nil, failure), "remaining": session.RemainingBudget(), "has_id": session.state.captchaId != "", "id_len": len(session.state.captchaId), "fetches": fetches, "points": points, "urls": urls})
				}
				value = observations
			case "locator":
				locator := NewJournalLocator(observation.Titles, observation.Issns)
				value = map[string]any{"titles": locator.Titles(), "issns": locator.Issns()}
			default:
				t.Fatalf("unknown operation %s", observation.Kind)
			}
			result := map[string]any{"ok": value}
			if err != nil {
				var failure *Error
				if !errors.As(err, &failure) {
					t.Fatal(err)
				}
				result = map[string]any{"error": map[string]any{"kind": failure.Kind, "display": failure.Error()}}
			}
			encoded, err := json.Marshal(result)
			if err != nil {
				t.Fatal(err)
			}
			decoder := json.NewDecoder(bytes.NewReader(encoded))
			decoder.UseNumber()
			var actual any
			if err := decoder.Decode(&actual); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(actual, observation.Output) {
				want, _ := json.Marshal(observation.Output)
				t.Fatalf("actual %s\nwant   %s", encoded, want)
			}
		})
	}
}

func cnkiOutcome(value any, err error) map[string]any {
	if err == nil {
		return map[string]any{"ok": value}
	}
	var failure *Error
	if !errors.As(err, &failure) {
		panic(err)
	}
	return map[string]any{"error": map[string]any{"kind": failure.Kind, "display": failure.Error()}}
}
func fixtureSequence(t *testing.T, data FixtureData, operations []map[string]any) []any {
	t.Helper()
	fixture := NewFixtureTransport(data)
	results := []any{}
	for _, operation := range operations {
		var value any
		var err error
		switch operation["op"] {
		case "resolve":
			titles := []string{}
			issns := []string{}
			for _, item := range operation["titles"].([]any) {
				titles = append(titles, item.(string))
			}
			for _, item := range operation["issns"].([]any) {
				issns = append(issns, item.(string))
			}
			value, err = fixture.ResolveJournal(context.Background(), NewJournalLocator(titles, issns))
		case "years":
			value, err = fixture.YearIssues(context.Background(), operation["journal"])
		case "papers":
			var page uint64
			if json.Unmarshal([]byte(operation["page"].(json.Number)), &page) != nil {
				t.Fatal("invalid page")
			}
			value, err = fixture.IssueArticles(context.Background(), operation["journal"], operation["issue"], page)
		case "article":
			value, err = fixture.ArticleDetail(context.Background(), operation["url"].(string), jsonText(operation["platform_id"]))
		case "reset":
			err = fixture.ResetTransientState(context.Background())
		case "drain":
			value = fixture.DrainAttempts()
		default:
			t.Fatal(operation)
		}
		results = append(results, map[string]any{"result": cnkiOutcome(value, err), "attempts": fixture.Attempts()})
	}
	return results
}
