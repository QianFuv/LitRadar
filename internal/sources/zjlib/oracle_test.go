package zjlib

import (
	"context"
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"testing"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func TestOriginalRustObservations(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/sources/zjlib-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []json.RawMessage `json:"observations"`
	}
	if err = json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, raw := range corpus.Observations {
		value, err := transport.ParseJson(raw)
		if err != nil {
			t.Fatal(err)
		}
		request := value.(map[string]any)
		t.Run(stringField(request, "id"), func(t *testing.T) {
			expected, err := transport.ParseJson([]byte(stringField(request, "output")))
			if err != nil {
				t.Fatal(err)
			}
			actual := observeOriginal(t, request)
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := transport.ParseJson(encoded)
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(decoded, expected) {
				t.Fatalf("want %s\ngot %s", stringField(request, "output"), encoded)
			}
		})
	}
}
func observeOriginal(t *testing.T, request map[string]any) any {
	text := stringField(request, "text")
	now := int64(1800000000)
	if instant := int64Field(request["now"]); instant != nil {
		now = *instant
	}
	switch stringField(request, "kind") {
	case "redirect_wire":
		return observeRedirectWire(t, request)
	case "forms":
		result := [][2]string{}
		for _, pair := range searchResultFields(text) {
			result = append(result, [2]string{pair.Name, pair.Value})
		}
		handler := [][2]string{}
		for _, pair := range searchHandlerFields(text, 0) {
			if pair.Name != "__" {
				handler = append(handler, [2]string{pair.Name, pair.Value})
			}
		}
		return map[string]any{"result": result, "handler": handler}
	case "javascript":
		return map[string]any{"decoded": decodeJsString(text), "sign": extractJsVar(text, "sign"), "url": extractJsVar(text, "url")}
	case "location":
		value, err := extractWindowLocation(text, stringField(request, "base"))
		return observedOutcome(value, err)
	case "sync":
		value, err := extractShareCookieSync(text, defaultEndpoints())
		if err != nil || value == nil {
			return observedOutcome(nil, err)
		}
		return observedOutcome(map[string]any{"url": value.url, "fields": value.fields}, nil)
	case "html":
		fallback := "fallback"
		if value, ok := request["fallback"].(string); ok {
			fallback = value
		}
		return ExtractArticleIdentity(text, fallback)
	case "search":
		value, err := ParseSearchResults(text, stringField(request, "base"))
		return observedOutcome(value, err)
	case "pdf":
		value, err := extractDownloadUrl(text, stringField(request, "base"), defaultEndpoints(), true)
		return observedOutcome(value, err)
	case "url":
		family := map[string]endpointFamily{"Www": wwwFamily, "Share": shareFamily, "ZyproxyLogin": loginFamily, "Zyproxy": proxyFamily}[stringField(request, "family")]
		if base, ok := request["base"].(string); ok {
			value, err := defaultEndpoints().join(base, text, family)
			return observedOutcome(value, err)
		}
		value, err := defaultEndpoints().parse(text, family)
		if err != nil {
			return observedOutcome(nil, err)
		}
		return observedOutcome(value.Href(false), nil)
	case "text":
		return map[string]any{"decoded": decodeHtml(text), "stripped": stripTags(text), "clean": cleanText(text), "normalized": normalizeExact(text), "filename": SafeFilename(text), "authors": splitAuthors(text)}
	case "jwt":
		if expiry := jwtExpiration(text); expiry != nil {
			return strconv.FormatInt(*expiry, 10)
		}
		return nil
	case "cookie":
		if cookie := CookieFromJson(request["value"]); cookie != nil {
			return map[string]any{"value": cookie.Json(), "unexpired": cookie.IsUnexpired(now)}
		}
		return nil
	case "identity":
		return MatchesArticle(observedIdentity(request["expected"]), observedIdentity(request["actual"]))
	case "mode":
		mode, ok := ParseFixtureMode(text)
		if !ok {
			return nil
		}
		return map[FixtureMode]string{Success: "Success", StartFailure: "StartFailure", PollTimeout: "PollTimeout", PollFailure: "PollFailure", WarmupFailure: "WarmupFailure", FulltextMismatch: "FulltextMismatch", FulltextFailure: "FulltextFailure"}[mode]
	case "sequence":
		mode, _ := ParseFixtureMode(stringField(request, "mode"))
		fixture := NewFixtureTransport(mode)
		fixture.state.now = func() int64 { return now }
		client := NewClient(fixture)
		client.state.now = func() int64 { return now }
		client.LoadStateData(request["state"])
		observations := []any{}
		for _, entry := range request["operations"].([]any) {
			operation := entry.(map[string]any)
			if instant := int64Field(operation["now"]); instant != nil {
				now = *instant
			}
			var result any
			var err error
			switch stringField(operation, "op") {
			case "save":
			case "load":
				client.LoadStateData(operation["state"])
			case "start":
				result, err = client.StartQrLogin(context.Background())
			case "poll":
				timeout := int64(180)
				if specified := int64Field(operation["timeout"]); specified != nil {
					timeout = *specified
				}
				result, err = client.PollQrLogin(context.Background(), timeout, 2)
			case "warm":
				result, err = client.WarmUpFulltextSession(context.Background())
			case "fresh":
				result = client.HasFreshFulltextSession(int64Field(operation["at"]))
			case "download":
				limit := uint64(10)
				if specified := int64Field(operation["limit"]); specified != nil {
					limit = uint64(*specified)
				}
				var pdf DownloadedPdf
				pdf, err = client.DownloadMatchingPdf(context.Background(), observedIdentity(operation["identity"]), limit)
				result = map[string]any{"filename": pdf.Filename, "final_url": pdf.FinalUrl, "content_type": pdf.ContentType, "byte_count": pdf.ByteCount, "content": string(pdf.Content)}
			default:
				t.Fatal("unknown operation")
			}
			outcome := map[string]any{"ok": result}
			if err != nil {
				outcome = map[string]any{"error": map[string]any{"kind": err.(*Error).Kind, "display": err.Error()}}
			}
			observations = append(observations, map[string]any{"result": outcome, "state": client.StateData()})
		}
		return observations
	default:
		t.Fatal("unknown observation")
		return nil
	}
}
func observedIdentity(value any) ArticleIdentity {
	return ArticleIdentity{Title: stringField(value, "title"), Authors: stringField(value, "authors"), JournalTitle: stringField(value, "journal_title")}
}

func observedOutcome(value any, err error) any {
	if err != nil {
		return map[string]any{"error": map[string]any{"kind": err.(*Error).Kind, "display": err.Error()}}
	}
	return map[string]any{"ok": value}
}
