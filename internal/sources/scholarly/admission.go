package scholarly

import (
	"context"
	"log/slog"
	"math"
	"sync"
	"time"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func unixScheduleTime() scheduleTime {
	value := transport.MonotonicTime().UnixMilli()
	if value < 0 {
		return scheduleTime{}
	}
	return milliseconds(uint64(value))
}
func sourceDeadlineError(service string) error {
	return &Error{Kind: "Request", Service: service, Endpoint: "request", Message: "source request deadline expired"}
}
func waitForStart(ctx context.Context, start scheduleTime, deadline time.Time) bool {
	for {
		wait := start.subtract(unixScheduleTime())
		if wait == (scheduleTime{}) {
			_, hasTime := transport.Remaining(deadline)
			return hasTime && ctx.Err() == nil
		}
		if !transport.RetrySleep(ctx, wait.timer(), deadline) {
			return false
		}
	}
}

type sharedOpenAlex struct {
	mutex   sync.Mutex
	state   *openAlexScheduler
	changed chan struct{}
}
type openAlexLease struct {
	owner *sharedOpenAlex
	slot  reservation
	once  sync.Once
}

func (scheduler *sharedOpenAlex) notify() {
	close(scheduler.changed)
	scheduler.changed = make(chan struct{})
}
func (scheduler *sharedOpenAlex) reserve(ctx context.Context, excluded []int, deadline time.Time) (*openAlexLease, error) {
	for {
		remaining, hasTime := transport.Remaining(deadline)
		if !hasTime || ctx.Err() != nil {
			return nil, sourceDeadlineError(OpenAlex)
		}
		scheduler.mutex.Lock()
		now := unixScheduleTime()
		choice := scheduler.state.reserve(now, excluded)
		changed := scheduler.changed
		if choice.Kind == "Unavailable" {
			hasEnabled := false
			for _, slot := range scheduler.state.Slots {
				hasEnabled = hasEnabled || !slot.IsDisabled
			}
			scheduler.mutex.Unlock()
			if hasEnabled {
				return nil, &Error{Kind: "Request", Service: OpenAlex, Endpoint: "admission", Message: "OpenAlex quota is temporarily unavailable."}
			}
			return nil, &Error{Kind: "Configuration", Message: "No eligible OpenAlex API key is available."}
		}
		scheduler.mutex.Unlock()
		if choice.Kind == "Reserved" {
			lease := &openAlexLease{owner: scheduler, slot: choice.Reservation}
			if !waitForStart(ctx, choice.Reservation.Start, deadline) {
				lease.cancel()
				return nil, sourceDeadlineError(OpenAlex)
			}
			scheduler.mutex.Lock()
			isEligible := scheduler.state.eligible(choice.Reservation, unixScheduleTime())
			scheduler.mutex.Unlock()
			if !isEligible {
				lease.cancel()
				continue
			}
			return lease, nil
		}
		wait := remaining
		if choice.Kind == "WaitUntil" {
			wait = choice.Until.subtract(now).timer()
			if wait <= 0 {
				continue
			}
			if wait >= remaining {
				return nil, sourceDeadlineError(OpenAlex)
			}
			if wait >= time.Second {
				slog.InfoContext(ctx, "source quota wait", "event", "source.openalex.quota_wait", "component", "source", "provider", OpenAlex, "reason", "quota_or_cooldown", "wait_ms", uint64(wait/time.Millisecond), "key_slot_count", len(scheduler.state.Slots))
			}
		}
		timer := time.NewTimer(wait)
		select {
		case <-changed:
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			return nil, sourceDeadlineError(OpenAlex)
		}
		timer.Stop()
	}
}
func (lease *openAlexLease) cancel() {
	lease.once.Do(func() {
		lease.owner.mutex.Lock()
		defer lease.owner.mutex.Unlock()
		lease.owner.state.cancel(lease.slot)
		lease.owner.notify()
	})
}
func (lease *openAlexLease) finish(headers rateHeaders, outcome health, delay scheduleTime, isSearch bool) {
	lease.once.Do(func() {
		lease.owner.mutex.Lock()
		defer lease.owner.mutex.Unlock()
		lease.owner.state.finish(lease.slot, unixScheduleTime(), headers, outcome, delay, isSearch)
		lease.owner.notify()
	})
}
func (lease *openAlexLease) observeThrottle(delay scheduleTime) {
	lease.owner.mutex.Lock()
	defer lease.owner.mutex.Unlock()
	extendCooldown(&lease.owner.state.Slots[lease.slot.Slot], unixScheduleTime().add(delay))
	lease.owner.notify()
}

func reserveSemantic(ctx context.Context, state *semanticScheduler, excluded []int, deadline time.Time) (reservation, error) {
	for {
		if _, hasTime := transport.Remaining(deadline); !hasTime || ctx.Err() != nil {
			return reservation{}, sourceDeadlineError(SemanticScholar)
		}
		choice := state.reserve(unixScheduleTime(), excluded)
		switch choice.Kind {
		case "Reserved":
			if !waitForStart(ctx, choice.Reservation.Start, deadline) {
				return reservation{}, sourceDeadlineError(SemanticScholar)
			}
			if state.obsolete(choice.Reservation, unixScheduleTime()) {
				continue
			}
			return choice.Reservation, nil
		case "WaitUntil":
			if !waitForStart(ctx, choice.Until, deadline) {
				return reservation{}, sourceDeadlineError(SemanticScholar)
			}
		default:
			return reservation{}, &Error{Kind: "Configuration", Message: "No eligible Semantic Scholar API key is available."}
		}
	}
}

type crossrefSchedule struct {
	Next, Period uint64
	Cooldown     scheduleTime
}

func saturatingAdd(first, second uint64) uint64 {
	if first > math.MaxUint64-second {
		return math.MaxUint64
	}
	return first + second
}
func newCrossrefSchedule(config LiveConfig) crossrefSchedule {
	count := max(config.SemanticScholarProcessCount, 1)
	worker := min(config.SemanticScholarWorkerId, count-1)
	return crossrefSchedule{Next: saturatingAdd(config.ScheduleEpochUnixMillis, saturatingMultiply(110, worker)), Period: saturatingMultiply(110, count)}
}
func (schedule *crossrefSchedule) reserve(now uint64) uint64 {
	if schedule.Period == 0 {
		return now
	}
	slot := schedule.Next
	if slot < now {
		overdue := now - slot
		skipped := overdue / schedule.Period
		if overdue%schedule.Period != 0 {
			skipped++
		}
		slot = saturatingAdd(slot, saturatingMultiply(skipped, schedule.Period))
	}
	schedule.Next = saturatingAdd(slot, schedule.Period)
	return slot
}
func (schedule *crossrefSchedule) wait(ctx context.Context, deadline time.Time) bool {
	ready := later(unixScheduleTime(), schedule.Cooldown)
	millis := saturatedUint64(ready.millis())
	if ready.Nanoseconds%1_000_000 != 0 {
		millis = saturatingIncrement(millis)
	}
	return waitForStart(ctx, later(milliseconds(schedule.reserve(millis)), schedule.Cooldown), deadline)
}
func (schedule *crossrefSchedule) deferFor(delay scheduleTime) {
	schedule.Cooldown = later(schedule.Cooldown, unixScheduleTime().add(delay))
}
