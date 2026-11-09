package scholarly

import (
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"
)

// referenceDoiBatches retains the full request encoding used before incremental sizing.
func referenceDoiBatches(dois []string, batchSize int, apiKey *string) ([][]string, error) {
	batches := [][]string{}
	batch := []string{}
	batchSize = max(1, min(batchSize, 100))
	for _, doi := range dois {
		candidate := append(append([]string{}, batch...), doi)
		if len(candidate) > batchSize || len("https://api.openalex.org/works?"+EncodeQuery(OpenAlexDoiQuery(candidate, apiKey))) > 1900 {
			if len(batch) > 0 {
				batches = append(batches, batch)
			}
			batch = []string{doi}
			if len("https://api.openalex.org/works?"+EncodeQuery(OpenAlexDoiQuery(batch, apiKey))) > 1900 {
				return nil, &Error{Kind: "Configuration", Message: "One OpenAlex DOI enrichment value exceeds the request URL budget."}
			}
		} else {
			batch = candidate
		}
	}
	if len(batch) > 0 {
		batches = append(batches, batch)
	}
	return batches, nil
}

// assertDoiBatches compares exact boundaries, byte encoding, and error publication with the reference.
func assertDoiBatches(t *testing.T, dois []string, batchSize int, apiKey *string) {
	t.Helper()
	want, expectedError := referenceDoiBatches(dois, batchSize, apiKey)
	actual, err := PartitionOpenAlexDois(dois, batchSize, apiKey)
	if !reflect.DeepEqual(actual, want) || (err == nil) != (expectedError == nil) {
		t.Fatalf("batch mismatch: actual=%q, error=%v; expected=%q, error=%v", actual, err, want, expectedError)
	}
	if expectedError != nil {
		var failure *Error
		if !errors.As(err, &failure) || failure.Kind != "Configuration" || err.Error() != expectedError.Error() || actual != nil {
			t.Fatal("oversized value changed failure classification or published partial batches", err)
		}
		return
	}
	for _, batch := range actual {
		if len(batch) > max(1, min(batchSize, 100)) || len("https://api.openalex.org/works?"+EncodeQuery(OpenAlexDoiQuery(batch, apiKey))) > 1900 {
			t.Fatal("batch exceeds upstream count or encoded byte budget")
		}
	}
}

// TestDoiBatchBoundaries covers inclusive URL limits, decimal counts, credentials, and raw bytes.
func TestDoiBatchBoundaries(t *testing.T) {
	for _, key := range []*string{nil, new(""), new("key+%|:*~ 中文"), new(strings.Repeat("k", 1901))} {
		baseLength := len("https://api.openalex.org/works?" + EncodeQuery(OpenAlexDoiQuery(nil, key)))
		for _, values := range [][]string{
			nil, {}, {""}, {"10.1/a", "10.1/a", "", "中文+%|:*~", string([]byte{0xff, 0xfe, ' '})},
			strings.Split(strings.Repeat("\x00", 224), "\x00"),
			{strings.Repeat("x", max(0, 1900-baseLength))},
			{"short", strings.Repeat("x", max(0, 1901-baseLength))},
			{strings.Repeat("%", 700)},
		} {
			for _, size := range []int{-1, 0, 1, 9, 10, 99, 100, 101} {
				assertDoiBatches(t, values, size, key)
			}
		}
	}
	for _, count := range []int{9, 10, 99, 100} {
		values := make([]string, count)
		baseLength := len("https://api.openalex.org/works?" + EncodeQuery(OpenAlexDoiQuery(values, nil)))
		values[0] = strings.Repeat("x", 1900-baseLength)
		assertDoiBatches(t, values, 100, nil)
		values[0] += "x"
		assertDoiBatches(t, values, 100, nil)
	}
}

// TestDoiBatchStorageIndependence prevents batches from sharing mutable slice storage with inputs or siblings.
func TestDoiBatchStorageIndependence(t *testing.T) {
	input := []string{"first", "second", "third", "fourth"}
	batches, err := PartitionOpenAlexDois(input, 2, nil)
	if err != nil || len(batches) != 2 {
		t.Fatal(batches, err)
	}
	input[0] = "input changed"
	batches[0][1] = "batch changed"
	appended := append(batches[0], "appended")
	if batches[0][0] != "first" || input[1] != "second" || batches[1][0] != "third" || appended[2] != "appended" {
		t.Fatal("returned batches alias input or another batch", input, batches)
	}
}

// FuzzDoiBatchEncoding checks exact partition compatibility across arbitrary byte sequences.
func FuzzDoiBatchEncoding(f *testing.F) {
	f.Add("10.1/a\x00中文+%|:*~\x00", "", 100, false)
	f.Add(strings.Repeat("\x00", 224), "encoded credential %~", 99, true)
	f.Add(string([]byte{0xff, 0, 0xfe}), "", 0, true)
	f.Fuzz(func(t *testing.T, input, key string, size int, hasKey bool) {
		if len(input) > 8192 || len(key) > 2048 {
			t.Skip()
		}
		var apiKey *string
		if hasKey {
			apiKey = &key
		}
		assertDoiBatches(t, strings.Split(input, "\x00"), size, apiKey)
	})
}

// BenchmarkDoiBatchPartition measures the preparation cost without making upstream requests.
func BenchmarkDoiBatchPartition(b *testing.B) {
	for _, count := range []int{225, 1000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			values := make([]string, count)
			for index := range values {
				values[index] = fmt.Sprintf("10.1234/synthetic.%d", index)
			}
			b.ReportAllocs()
			for b.Loop() {
				batches, err := PartitionOpenAlexDois(values, 100, new("synthetic key"))
				if err != nil || len(batches) == 0 {
					b.Fatal(batches, err)
				}
			}
		})
	}
}
