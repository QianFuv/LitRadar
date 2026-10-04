// Package index defines provider-independent indexing execution limits.
package index

import (
	"errors"
	"fmt"
)

// Concurrency is the validated configured source capacity across journal processes.
type Concurrency struct{ WorkerCount, ProcessCount, AggregateCapacity uint64 }

// ValidateConcurrencyOptions validates explicitly supplied counts before provider selection.
func ValidateConcurrencyOptions(workers, processes *uint64) error {
	if workers != nil && (*workers < 1 || *workers > 32) {
		return errors.New("worker_count must be between 1 and 32")
	}
	if processes != nil && (*processes < 1 || *processes > 32) {
		return errors.New("process_count must be between 1 and 32")
	}
	return nil
}

// ValidateConcurrency enforces provider-specific process and aggregate limits.
func ValidateConcurrency(workers, processes uint64, isScholarly bool) (Concurrency, error) {
	if err := ValidateConcurrencyOptions(&workers, &processes); err != nil {
		return Concurrency{}, err
	}
	if isScholarly && processes > 3 {
		return Concurrency{}, errors.New("process_count must be at most 3 for scholarly indexing")
	}
	limit := uint64(32)
	if isScholarly {
		limit = 96
	}
	capacity := workers * processes
	if capacity > limit {
		return Concurrency{}, fmt.Errorf("process_count * worker_count must be at most %d", limit)
	}
	return Concurrency{workers, processes, capacity}, nil
}

// ResolveConcurrency applies each omitted default independently for the selected provider.
func ResolveConcurrency(workers, processes *uint64, isScholarly bool) (Concurrency, error) {
	selectedWorkers, selectedProcesses := uint64(6), uint64(1)
	if isScholarly {
		selectedProcesses = 3
	}
	if workers != nil {
		selectedWorkers = *workers
	}
	if processes != nil {
		selectedProcesses = *processes
	}
	return ValidateConcurrency(selectedWorkers, selectedProcesses, isScholarly)
}
