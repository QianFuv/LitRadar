package sources

import (
	"testing"

	"github.com/QianFuv/LitRadar/internal/sources/cnki"
)

func TestCnkiRetryCacheDropsSuccessAfterFinalFailure(t *testing.T) {
	index, state := newCnkiIndexTest(t, []any{cnkiTestRow("A"), cnkiTestRow("B")}, 1)
	calls := map[string]int{}
	shouldFail := true
	state.detail = func(url string) (any, error) {
		calls[url]++
		if url == "B" && shouldFail {
			return nil, &cnki.Error{Kind: "Request", Message: "temporary"}
		}
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	if _, err := cnkiTestFetch(index, nil); err == nil {
		t.Fatal("final detail failure was ignored")
	}
	if calls["A"] != 1 || calls["B"] != 3 {
		t.Fatal(calls)
	}
	shouldFail = false
	batch, err := cnkiTestFetch(index, nil)
	if err != nil || len(batch.Articles) != 2 || calls["A"] != 2 || calls["B"] != 4 {
		t.Fatalf("articles=%d calls=%v err=%v", len(batch.Articles), calls, err)
	}
}

func TestCnkiRetryCacheNeverRetainsConversionFailure(t *testing.T) {
	firstRow := cnkiTestRow("A").(map[string]any)
	firstRow["title"] = nil
	index, state := newCnkiIndexTest(t, []any{firstRow, cnkiTestRow("B")}, 1)
	calls := map[string]int{}
	state.detail = func(url string) (any, error) {
		calls[url]++
		if url == "A" && calls[url] == 1 {
			return map[string]any{"title": nil, "doi": "10.1000/conversion", "authors": "Author"}, nil
		}
		return map[string]any{"title": url, "authors": "Author"}, nil
	}
	batch, err := cnkiTestFetch(index, nil)
	if err != nil || len(batch.Articles) != 2 || calls["A"] != 2 || calls["B"] != 1 {
		t.Fatalf("articles=%d calls=%v err=%v", len(batch.Articles), calls, err)
	}
}
