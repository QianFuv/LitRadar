package query

import (
	"os"
	"sync"
)

// relatedCostCacheLimit bounds each of the at most sixteen database cost maps.
const relatedCostCacheLimit = 4096

// relatedCostKey separates capped counts computed under different recall budgets.
type relatedCostKey struct {
	term   string
	budget int
}

// relatedDatabaseCosts retains bounded term costs for one observed database generation.
type relatedDatabaseCosts struct {
	sync.Mutex
	file   os.FileInfo
	values map[relatedCostKey]int
}

// relatedCounts owns file identities as well as costs, so replacing an index at the
// same pathname cannot reuse the previous index's document frequencies.
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
		cache = &relatedDatabaseCosts{file: info, values: map[relatedCostKey]int{}}
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
