package telemetry

import (
	"context"
	"encoding/json"
	"sort"
	"time"
)

// Summary merges a dimension's hourly histograms before estimating percentiles.
type Summary struct {
	Dimensions
	Calls     uint64  `json:"calls"`
	Errors    uint64  `json:"errors"`
	ErrorRate float64 `json:"error_rate"`
	P50Ms     float64 `json:"p50_ms"`
	P95Ms     float64 `json:"p95_ms"`
	P99Ms     float64 `json:"p99_ms"`
}

// Hour contains observed calls and errors for one UTC hour and kind.
type Hour struct {
	Hour   time.Time `json:"hour"`
	Kind   string    `json:"kind"`
	Calls  uint64    `json:"calls"`
	Errors uint64    `json:"errors"`
}

// Distribution counts a search shape feature's calls in the selected range.
type Distribution struct {
	Feature string `json:"feature"`
	Value   string `json:"value"`
	Calls   uint64 `json:"calls"`
}

// Report includes durable and pending observations; ranges are half-open UTC hours.
type Report struct {
	Enabled      bool           `json:"enabled"`
	From         time.Time      `json:"from"`
	To           time.Time      `json:"to"`
	Series       []Summary      `json:"series"`
	Hourly       []Hour         `json:"hourly"`
	SearchShapes []Distribution `json:"search_shapes"`
}

// Read takes a consistent persisted-plus-pending snapshot without blocking recorders on I/O.
func (collector *Collector) Read(ctx context.Context, from, to time.Time, kind string) (Report, error) {
	report := Report{Enabled: collector != nil, From: from, To: to, Series: []Summary{}, Hourly: []Hour{}, SearchShapes: []Distribution{}}
	if collector == nil {
		return report, nil
	}
	collector.storeMutex.Lock()
	defer collector.storeMutex.Unlock()
	buckets := map[bucketKey]aggregate{}
	if collector.store != nil {
		rows, err := collector.store.database.QueryContext(ctx, "SELECT hour,dimensions,aggregate FROM hourly_load WHERE hour>=? AND hour<?", from.Unix(), to.Unix())
		if err != nil {
			return report, err
		}
		defer rows.Close()
		for rows.Next() {
			var key bucketKey
			var labels, encoded string
			var value aggregate
			if err := rows.Scan(&key.Hour, &labels, &encoded); err != nil {
				return report, err
			}
			if err := json.Unmarshal([]byte(labels), &key.Dimensions); err != nil {
				return report, err
			}
			if err := json.Unmarshal([]byte(encoded), &value); err != nil {
				return report, err
			}
			buckets[key] = value
		}
		if err := rows.Err(); err != nil {
			return report, err
		}
	}
	collector.mutex.Lock()
	for key, value := range collector.pending {
		if key.Hour >= from.Unix() && key.Hour < to.Unix() {
			combined := buckets[key]
			combined.merge(value)
			buckets[key] = combined
		}
	}
	collector.mutex.Unlock()
	report.summarize(buckets, kind)
	return report, nil
}

// summarize produces deterministic summaries, sparse hourly counts and marginal shape distributions.
func (report *Report) summarize(buckets map[bucketKey]aggregate, kind string) {
	series := map[Dimensions]aggregate{}
	type hourKey struct {
		hour int64
		kind string
	}
	hours := map[hourKey]aggregate{}
	type featureKey struct{ feature, value string }
	features := map[featureKey]uint64{}
	for key, value := range buckets {
		if kind != "" && key.Kind != kind {
			continue
		}
		combined := series[key.Dimensions]
		combined.merge(value)
		series[key.Dimensions] = combined
		hour := hourKey{key.Hour, key.Kind}
		hourValue := hours[hour]
		hourValue.merge(value)
		hours[hour] = hourValue
		if key.Kind == "search" {
			for feature, label := range map[string]string{"endpoint": key.Name, "mode": key.Mode, "database": key.Database, "term_count": key.TermCount, "uses_prefix": boolLabel(key.UsesPrefix), "uses_boolean": boolLabel(key.UsesBoolean), "hits": key.Hits, "hit_count_basis": key.HitCountBasis} {
				features[featureKey{feature, label}] += value.Calls
			}
		}
	}
	for dimensions, value := range series {
		report.Series = append(report.Series, Summary{Dimensions: dimensions, Calls: value.Calls, Errors: value.Errors, ErrorRate: float64(value.Errors) / float64(value.Calls), P50Ms: value.percentile(50), P95Ms: value.percentile(95), P99Ms: value.percentile(99)})
	}
	sort.Slice(report.Series, func(first, second int) bool {
		left, right := report.Series[first], report.Series[second]
		if left.Calls != right.Calls {
			return left.Calls > right.Calls
		}
		leftKey, _ := json.Marshal(left.Dimensions)
		rightKey, _ := json.Marshal(right.Dimensions)
		return string(leftKey) < string(rightKey)
	})
	for key, value := range hours {
		report.Hourly = append(report.Hourly, Hour{time.Unix(key.hour, 0).UTC(), key.kind, value.Calls, value.Errors})
	}
	sort.Slice(report.Hourly, func(first, second int) bool {
		left, right := report.Hourly[first], report.Hourly[second]
		if !left.Hour.Equal(right.Hour) {
			return left.Hour.Before(right.Hour)
		}
		return left.Kind < right.Kind
	})
	for key, calls := range features {
		report.SearchShapes = append(report.SearchShapes, Distribution{key.feature, key.value, calls})
	}
	sort.Slice(report.SearchShapes, func(first, second int) bool {
		left, right := report.SearchShapes[first], report.SearchShapes[second]
		if left.Feature != right.Feature {
			return left.Feature < right.Feature
		}
		return left.Value < right.Value
	})
}

// boolLabel emits a fixed distribution label.
func boolLabel(value bool) string {
	if value {
		return "true"
	}
	return "false"
}
