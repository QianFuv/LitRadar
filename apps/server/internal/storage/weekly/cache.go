package weekly

import (
	"container/list"
	"os"
	"path/filepath"
	"slices"
	"sync"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/rootfile"
)

const cacheCapacity = 64
const cacheArticleLimit = 1000000
const cacheTTL = 60 * time.Second

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
	element     *list.Element
}

// Cache bounds parsed publication retention without hiding filesystem or parsing failures.
type Cache struct {
	mutex         sync.Mutex
	entries       map[string]cacheEntry
	articleIds    int
	parseAttempts uint64
	order         list.List
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
	if value.RunID != nil {
		text := *value.RunID
		copy.RunID = &text
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
		cache.order.Remove(entry.element)
		delete(cache.entries, path)
	}
}
func (cache *Cache) prune(now time.Time) {
	for path, entry := range cache.entries {
		if now.Sub(entry.loadedAt) >= cacheTTL {
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
	root, name, err := rootfile.Parent(filename)
	if err != nil {
		return nil, err
	}
	defer func() { _ = root.Close() }()
	return cache.readRootAt(root, name, now)
}

func (cache *Cache) readRootAt(root *os.Root, name string, now time.Time) (*Manifest, error) {
	path := filepath.Join(root.Name(), name)
	before, err := rootFingerprint(root, name)
	if err != nil {
		cache.mutex.Lock()
		cache.entries = nil
		cache.order.Init()
		cache.articleIds = 0
		cache.mutex.Unlock()
		return nil, err
	}
	cache.mutex.Lock()
	result, hit := cache.lookupLocked(path, before, now)
	cache.mutex.Unlock()
	if hit {
		return result, nil
	}
	parsed, err := readRootManifest(root, name)
	if err != nil {
		return nil, err
	}
	count := articleCount(parsed)
	after, err := rootFingerprint(root, name)
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
	if entry, exists := cache.entries[path]; exists && entry.fingerprint == before {
		cache.order.MoveToBack(entry.element)
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
	cache.entries[path] = cacheEntry{before, cloneManifest(parsed), now, cache.order.PushBack(path)}
	cache.articleIds += count
}

// evictLocked enforces entry and article limits in least-recently-used order.
func (cache *Cache) evictLocked(count int) {
	for len(cache.entries) >= cacheCapacity || cache.articleIds+count > cacheArticleLimit {
		oldest := cache.order.Front()
		if oldest == nil {
			break
		}
		cache.remove(oldest.Value.(string))
	}
}
