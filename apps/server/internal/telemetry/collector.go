// Package telemetry aggregates anonymous load measurements without request-time I/O.
package telemetry

import (
	"context"
	"math/bits"
	"sync"
	"sync/atomic"
	"time"
)

const seriesLimit = 8192

// Dimensions contains only server-controlled labels and coarse search features.
type Dimensions struct {
	Kind          string `json:"kind" nullable:"false" enum:"http,mcp,search"`
	Name          string `json:"name" nullable:"false"`
	Method        string `json:"method,omitempty" required:"false" nullable:"false"`
	StatusClass   string `json:"status_class,omitempty" required:"false" nullable:"false" enum:"2xx,3xx,4xx,5xx"`
	IsError       bool   `json:"is_error" nullable:"false"`
	Mode          string `json:"mode,omitempty" required:"false" nullable:"false" enum:"simple,advanced"`
	Database      string `json:"database,omitempty" required:"false" nullable:"false"`
	TermCount     string `json:"term_count,omitempty" required:"false" nullable:"false" enum:"0,1,2-4,5-9,10+"`
	UsesPrefix    bool   `json:"uses_prefix" nullable:"false"`
	UsesBoolean   bool   `json:"uses_boolean" nullable:"false"`
	Hits          string `json:"hits,omitempty" required:"false" nullable:"false" enum:"0,1-9,10-99,100-999,1000+"`
	HitCountBasis string `json:"hit_count_basis,omitempty" required:"false" nullable:"false" enum:"total,returned,candidates"`
}

type bucketKey struct {
	Hour int64
	Dimensions
}

type aggregate struct {
	Calls     uint64     `json:"calls"`
	Errors    uint64     `json:"errors"`
	Histogram [32]uint64 `json:"histogram"`
}

// Collector bounds series cardinality and drops observations on contention.
// Disk operations use a separate mutex and never hold the recording mutex.
type Collector struct {
	dropped       atomic.Uint64
	mutex         sync.Mutex
	pending       map[bucketKey]aggregate
	storeMutex    sync.Mutex
	store         *store
	retentionDays int
}

// New creates an enabled in-memory collector; a nil collector is disabled.
func New() *Collector { return &Collector{pending: make(map[bucketKey]aggregate), retentionDays: 90} }

// Record adds one measurement, dropping it if the bounded collector is busy.
// Callers must supply route templates, declared tool names or sanitized search shapes.
func (collector *Collector) Record(at time.Time, dimensions Dimensions, duration time.Duration) {
	if collector == nil {
		return
	}
	if !collector.mutex.TryLock() {
		collector.dropped.Add(1)
		return
	}
	defer collector.mutex.Unlock()
	key := bucketKey{at.UTC().Truncate(time.Hour).Unix(), dimensions}
	value, exists := collector.pending[key]
	if !exists && len(collector.pending) >= seriesLimit {
		collector.dropped.Add(1)
		return
	}
	value.Calls++
	if dimensions.IsError {
		value.Errors++
	}
	microseconds := max(int64(1), duration.Microseconds())
	if duration > time.Microsecond && duration%time.Microsecond != 0 {
		microseconds++
	}
	index := min(31, bits.Len64(uint64(microseconds)-1))
	value.Histogram[index]++
	collector.pending[key] = value
}

// HTTP records a normalized method, matched route template and status class.
func (collector *Collector) HTTP(route, method string, status int, duration time.Duration) {
	if collector == nil {
		return
	}
	class := "5xx"
	switch {
	case status < 300:
		class = "2xx"
	case status < 400:
		class = "3xx"
	case status < 500:
		class = "4xx"
	}
	collector.Record(time.Now(), Dimensions{Kind: "http", Name: route, Method: method, StatusClass: class, IsError: status >= 400}, duration)
}

// Mcp records a declared tool name and its protocol or tool-result failure flag.
func (collector *Collector) Mcp(name string, isError bool, duration time.Duration) {
	if collector == nil {
		return
	}
	collector.Record(time.Now(), Dimensions{Kind: "mcp", Name: name, IsError: isError}, duration)
}

type contextKey struct{}

// WithCollector attaches only the recorder; request identity is never inspected.
func WithCollector(ctx context.Context, collector *Collector) context.Context {
	if collector == nil {
		return ctx
	}
	return context.WithValue(ctx, contextKey{}, collector)
}

// FromContext returns the shared recorder, or nil when telemetry is disabled.
func FromContext(ctx context.Context) *Collector {
	collector, _ := ctx.Value(contextKey{}).(*Collector)
	return collector
}

// merge combines fixed logarithmic histograms before percentile calculation.
func (value *aggregate) merge(other aggregate) {
	value.Calls += other.Calls
	value.Errors += other.Errors
	for index := range value.Histogram {
		value.Histogram[index] += other.Histogram[index]
	}
}

// percentile returns the nearest-rank histogram upper bound in milliseconds.
func (value aggregate) percentile(percent uint64) float64 {
	if value.Calls == 0 {
		return 0
	}
	target, count := (value.Calls*percent+99)/100, uint64(0)
	for index, frequency := range value.Histogram {
		count += frequency
		if count >= target {
			return float64(uint64(1)<<index) / 1000
		}
	}
	return 0
}
