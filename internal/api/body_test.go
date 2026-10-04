package api

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http/httptest"
	"os"
	"strconv"
	"strings"
	"testing"
)

func TestJsonExtractionMatchesOriginalRawRequests(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/api/http-json-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		ExporterSha256 string `json:"exporter_sha256"`
		Cases          []struct {
			Type, Body  string
			ContentType *string `json:"content_type"`
			Status      int
			Response    string
		}
	}
	if err = json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	exporter, err := os.ReadFile("../../tests/migration/api/export-mcp.mjs")
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(exporter)
	if hex.EncodeToString(digest[:]) != corpus.ExporterSha256 {
		t.Fatal("stale original JSON observations")
	}
	if len(corpus.Cases) < 67 {
		t.Fatal("incomplete JSON corpus")
	}
	for index, scenario := range corpus.Cases {
		t.Run(strconv.Itoa(index), func(t *testing.T) {
			kind, exists := requestBodies[scenario.Type]
			if !exists {
				t.Fatal(scenario.Type)
			}
			request := httptest.NewRequest("POST", "/", strings.NewReader(scenario.Body))
			if scenario.ContentType != nil {
				request.Header.Set("Content-Type", *scenario.ContentType)
			}
			_, failure := extractBody(request, kind, scenario.Type == "AdminInviteCodeCreate")
			if scenario.Status == 401 {
				if failure != nil {
					t.Fatalf("decoded success expected: %s", failure.detail)
				}
				return
			}
			if failure == nil {
				t.Fatalf("wanted %d %s", scenario.Status, scenario.Response)
			}
			if failure.status != scenario.Status || failure.detail != scenario.Response {
				t.Fatalf("got %d %s; want %d %s", failure.status, failure.detail, scenario.Status, scenario.Response)
			}
		})
	}
}

func TestJsonBodyLimitAndOptionalAbsence(t *testing.T) {
	for _, size := range []int{2 * 1024 * 1024, 2*1024*1024 + 1} {
		body := "{}" + strings.Repeat(" ", size-2)
		request := httptest.NewRequest("POST", "/", strings.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		_, failure := extractBody(request, requestBodies["TokenCreateRequest"], false)
		if size == 2*1024*1024 && failure != nil {
			t.Fatal(failure)
		}
		if size > 2*1024*1024 && (failure == nil || failure.status != 413) {
			t.Fatal("oversize body not rejected")
		}
		request = httptest.NewRequest("POST", "/", strings.NewReader(body))
		_, failure = extractBody(request, requestBodies["AdminInviteCodeCreate"], true)
		if failure != nil {
			t.Fatal(failure)
		}
	}
}
