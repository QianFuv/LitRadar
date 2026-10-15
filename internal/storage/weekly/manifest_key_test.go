package weekly

import (
	"encoding/binary"
	"fmt"
	"math"
	"slices"
	"strconv"
	"strings"
	"testing"
)

// referenceManifestKey retains the original collision-free textual publication identity.
func referenceManifestKey(manifest Manifest) string {
	var result strings.Builder
	result.WriteString(strconv.Quote(manifest.DbName))
	result.WriteByte('|')
	if manifest.RunId == nil {
		result.WriteString("null")
	} else {
		result.WriteString(strconv.Quote(*manifest.RunId))
	}
	result.WriteByte('|')
	result.WriteString(strconv.FormatInt(manifest.GeneratedAt.Seconds, 10))
	result.WriteByte(':')
	result.WriteString(strconv.FormatUint(uint64(manifest.GeneratedAt.Nanoseconds), 10))
	for _, id := range manifest.ArticleIds {
		result.WriteByte('|')
		result.WriteString(strconv.FormatInt(id, 10))
	}
	return result.String()
}

// TestManifestKeyRetainsIdentityBoundaries distinguishes nullable runs, ordered IDs and extreme integers.
func TestManifestKeyRetainsIdentityBoundaries(t *testing.T) {
	base := Manifest{DbName: "alpha.sqlite", GeneratedAt: Timestamp{-1, math.MaxUint32}, ArticleIds: []int64{math.MinInt64, 0, math.MaxInt64}}
	variants := []Manifest{base, base, base, base, base}
	variants[1].RunId = new("null")
	variants[2].DbName = "alpha|\".sqlite"
	variants[3].ArticleIds = []int64{math.MaxInt64, 0, math.MinInt64}
	variants[4].GeneratedAt.Seconds = math.MaxInt64
	seen := map[string]bool{}
	for _, manifest := range variants {
		key := manifestKey(manifest)
		if key != referenceManifestKey(manifest) || seen[key] {
			t.Fatal("publication identities collided or changed", key)
		}
		seen[key] = true
	}
}

// FuzzManifestKey compares exact identity bytes for arbitrary names, timestamps and full-width IDs.
func FuzzManifestKey(f *testing.F) {
	f.Add("alpha.sqlite", "run", true, int64(-1), uint32(math.MaxUint32), []byte{0, 0, 0, 0, 0, 0, 0, 128})
	f.Add("\xff|\".sqlite", "null", false, int64(math.MaxInt64), uint32(0), []byte{})
	f.Fuzz(func(t *testing.T, database, run string, hasRun bool, seconds int64, nanos uint32, data []byte) {
		if len(data) > 8192 || len(database)+len(run) > 8192 {
			t.Skip()
		}
		ids := make([]int64, len(data)/8)
		for index := range ids {
			ids[index] = int64(binary.LittleEndian.Uint64(data[index*8:]))
		}
		before := slices.Clone(ids)
		manifest := Manifest{DbName: database, GeneratedAt: Timestamp{seconds, nanos}, ArticleIds: ids}
		if hasRun {
			manifest.RunId = &run
		}
		if actual, expected := manifestKey(manifest), referenceManifestKey(manifest); actual != expected || !slices.Equal(ids, before) {
			t.Fatal("publication key changed or mutated membership", actual, expected)
		}
	})
}

// BenchmarkManifestKey measures complete publication keys without per-ID parsing or fixture allocation.
func BenchmarkManifestKey(b *testing.B) {
	for _, count := range []int{1, 10000, 100000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			manifest := Manifest{DbName: "alpha.sqlite", RunId: new("run"), GeneratedAt: Timestamp{1791028800, 0}, ArticleIds: make([]int64, count)}
			for index := range manifest.ArticleIds {
				manifest.ArticleIds[index] = int64(index + 1)
			}
			expected := referenceManifestKey(manifest)
			b.ReportAllocs()
			for b.Loop() {
				if key := manifestKey(manifest); key != expected {
					b.Fatal("publication key changed")
				}
			}
		})
	}
}
