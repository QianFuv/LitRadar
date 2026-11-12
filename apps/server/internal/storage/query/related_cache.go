package query

import (
	"os"
	"slices"
	"sync"
)

// relatedCostCacheLimit bounds each of the at most sixteen database cost maps.
const relatedCostCacheLimit = 4096

// relatedCandidateCacheLimit bounds ranked candidate lists within one file generation.
const relatedCandidateCacheLimit = 256

// relatedCostKey separates capped counts computed under different recall budgets.
type relatedCostKey struct {
	term   string
	budget int
}

// relatedResultKey includes every input that changes ranked candidate membership or order.
type relatedResultKey struct {
	title string
	id    int64
	limit int64
}

// relatedDatabaseCosts retains bounded costs and candidate IDs for one database generation.
type relatedDatabaseCosts struct {
	sync.Mutex
	file    os.FileInfo
	values  map[relatedCostKey]int
	results map[relatedResultKey][]int64
}

// relatedCounts owns file identities, costs, and ranked candidates so replacing an
// index at the same pathname cannot reuse the previous generation.
var relatedCounts = struct {
	sync.Mutex
	files map[string]*relatedDatabaseCosts
}{files: map[string]*relatedDatabaseCosts{}}

// relatedCostsForFile bypasses caching while WAL or rollback-journal content can change
// independently of the main file. Empty WAL files contain no committed index changes.
func relatedCostsForFile(filename string) *relatedDatabaseCosts {
	info, err := os.Stat(filename)
	if err != nil {
		return nil
	}
	for _, suffix := range []string{"-wal", "-journal"} {
		journal, err := os.Stat(filename + suffix)
		if err == nil && journal.Size() > 0 || err != nil && !os.IsNotExist(err) {
			relatedCounts.Lock()
			delete(relatedCounts.files, filename)
			relatedCounts.Unlock()
			return nil
		}
	}
	relatedCounts.Lock()
	defer relatedCounts.Unlock()
	cache := relatedCounts.files[filename]
	if cache == nil || !os.SameFile(info, cache.file) || info.Size() != cache.file.Size() || !info.ModTime().Equal(cache.file.ModTime()) {
		if len(relatedCounts.files) >= 16 {
			clear(relatedCounts.files)
		}
		cache = &relatedDatabaseCosts{file: info, values: map[relatedCostKey]int{}, results: map[relatedResultKey][]int64{}}
		relatedCounts.files[filename] = cache
	}
	return cache
}

// get returns an exact cached count when this generation has already counted the term.
func (cache *relatedDatabaseCosts) get(term string, budget int) (int, bool) {
	if cache == nil {
		return 0, false
	}
	cache.Lock()
	defer cache.Unlock()
	cost, found := cache.values[relatedCostKey{term, budget}]
	return cost, found
}

// put keeps the existing bounded-map policy without introducing a custom LRU.
func (cache *relatedDatabaseCosts) put(term string, budget int, cost int) {
	if cache == nil {
		return
	}
	cache.Lock()
	defer cache.Unlock()
	if len(cache.values) >= relatedCostCacheLimit {
		clear(cache.values)
	}
	cache.values[relatedCostKey{term, budget}] = cost
}

// getResult returns owned candidate IDs so callers cannot mutate cached rankings.
func (cache *relatedDatabaseCosts) getResult(key relatedResultKey) ([]int64, bool) {
	if cache == nil {
		return nil, false
	}
	cache.Lock()
	defer cache.Unlock()
	ids, found := cache.results[key]
	return slices.Clone(ids), found
}

// putResult stores only completed rankings and follows the existing bounded-map policy.
func (cache *relatedDatabaseCosts) putResult(key relatedResultKey, ids []int64) {
	if cache == nil {
		return
	}
	cache.Lock()
	defer cache.Unlock()
	if len(cache.results) >= relatedCandidateCacheLimit {
		clear(cache.results)
	}
	cache.results[key] = slices.Clone(ids)
}
