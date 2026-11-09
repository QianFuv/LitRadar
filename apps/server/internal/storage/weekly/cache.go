package weekly

import (
	"slices"
	"sync"
	"time"
)

const cacheCapacity = 64
const cacheArticleLimit = 1000000
const cacheTtl = 60 * time.Second

type fileFingerprint struct {
	length, modifiedSeconds int64
	modifiedNanos           int
	created                 Timestamp
	hasCreated              bool
}
type cacheEntry struct {
	fingerprint fileFingerprint
	manifest    *Manifest
	loadedAt    time.Time
	lastAccess  uint64
}

// Cache bounds parsed publication retention without hiding filesystem or parsing failures.
type Cache struct {
	mutex                         sync.Mutex
	entries                       map[string]cacheEntry
	articleIds                    int
	parseAttempts, accessSequence uint64
}

// CacheStats exposes only non-content verification counters.
type CacheStats struct {
	Entries, ArticleIds int
	ParseAttempts       uint64
}

func cloneManifest(value *Manifest) *Manifest {
	if value == nil {
		return nil
	}
	copy := *value
	copy.ArticleIds = slices.Clone(value.ArticleIds)
	if value.RunId != nil {
		text := *value.RunId
		copy.RunId = &text
	}
	return &copy
}
func articleCount(value *Manifest) int {
	if value == nil {
		return 0
	}
	return len(value.ArticleIds)
}
func (cache *Cache) remove(path string) {
	if entry, exists := cache.entries[path]; exists {
		cache.articleIds -= articleCount(entry.manifest)
		delete(cache.entries, path)
	}
}
func (cache *Cache) prune(now time.Time) {
	for path, entry := range cache.entries {
		if now.Sub(entry.loadedAt) >= cacheTtl {
			cache.remove(path)
		}
	}
}

// Stats expires stale entries before reporting retained counts and cumulative parse attempts.
func (cache *Cache) Stats() CacheStats { return cache.statsAt(time.Now()) }
func (cache *Cache) statsAt(now time.Time) CacheStats {
	cache.mutex.Lock()
	defer cache.mutex.Unlock()
	cache.prune(now)
	return CacheStats{len(cache.entries), cache.articleIds, cache.parseAttempts}
}
func (cache *Cache) read(filename string) (*Manifest, error) {
	return cache.readAt(filename, time.Now())
}

func (cache *Cache) readAt(filename string, now time.Time) (*Manifest, error) {
	path, err := canonicalPath(filename)
	if err != nil {
		cache.mutex.Lock()
		cache.entries = nil
		cache.articleIds = 0
		cache.mutex.Unlock()
		return nil, err
	}
	before, err := fingerprint(path)
	if err != nil {
		cache.mutex.Lock()
		cache.remove(path)
		cache.mutex.Unlock()
		return nil, err
	}
	cache.mutex.Lock()
	result, hit := cache.lookupLocked(path, before, now)
	cache.mutex.Unlock()
	if hit {
		return result, nil
	}
	parsed, err := readManifest(path)
	if err != nil {
		return nil, err
	}
	count := articleCount(parsed)
	after, err := fingerprint(path)
	if count > cacheArticleLimit || err != nil || before != after {
		return parsed, nil
	}
	cache.mutex.Lock()
	defer cache.mutex.Unlock()
	cache.retainLocked(path, before, parsed, count, now)
	return parsed, nil
}

// lookupLocked updates LRU hits or counts a parse attempt while the caller owns the mutex.
func (cache *Cache) lookupLocked(path string, before fileFingerprint, now time.Time) (*Manifest, bool) {
	cache.prune(now)
	cache.accessSequence++
	if entry, exists := cache.entries[path]; exists && entry.fingerprint == before {
		entry.lastAccess = cache.accessSequence
		cache.entries[path] = entry
		result := cloneManifest(entry.manifest)
		return result, true
	}
	cache.remove(path)
	cache.parseAttempts++
	return nil, false
}

// retainLocked clones a stable publication under the caller's existing admission lock.
func (cache *Cache) retainLocked(path string, before fileFingerprint, parsed *Manifest, count int, now time.Time) {
	cache.prune(now)
	cache.remove(path)
	cache.evictLocked(count)
	if cache.entries == nil {
		cache.entries = map[string]cacheEntry{}
	}
	cache.accessSequence++
	cache.entries[path] = cacheEntry{before, cloneManifest(parsed), now, cache.accessSequence}
	cache.articleIds += count
}

// evictLocked enforces entry and article limits using the existing least-recent-access sequence.
func (cache *Cache) evictLocked(count int) {
	for len(cache.entries) >= cacheCapacity || cache.articleIds+count > cacheArticleLimit {
		oldest := ""
		var sequence uint64
		for key, entry := range cache.entries {
			if oldest == "" || entry.lastAccess < sequence {
				oldest, sequence = key, entry.lastAccess
			}
		}
		if oldest == "" {
			break
		}
		cache.remove(oldest)
	}
}
