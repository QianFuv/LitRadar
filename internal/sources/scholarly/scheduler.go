package scholarly

import (
	"math/big"
	"slices"
)

type health string

const (
	healthSuccess        health = "Success"
	healthAuthentication health = "AuthenticationFailure"
	healthRateLimited    health = "RateLimited"
	healthDailyLimited   health = "DailyQuotaLimited"
	healthTransient      health = "TransientFailure"
	healthTerminal       health = "TerminalFailure"
)

type reservation struct {
	Slot  int
	Start scheduleTime
}
type decision struct {
	Kind        string
	Reservation reservation
	Until       scheduleTime
}
type rateHeaders struct {
	Remaining, CreditsUsed *uint64
	ResetAfter, RetryAfter *scheduleTime
}
type keyState struct {
	Cooldown, Reset *scheduleTime
	Next            scheduleTime
	IsDisabled      bool
	Remaining       *uint64
	InFlight        uint64
	Credit          big.Int
}

type semanticScheduler struct {
	Slots            []keyState
	NextTie          int
	Interval, Period scheduleTime
}

func newSemanticScheduler(keyCount int, processId, processCount uint64, epoch, interval scheduleTime) *semanticScheduler {
	worker, count := processContext(processId, processCount)
	interval = later(interval, milliseconds(1))
	state := &semanticScheduler{Slots: make([]keyState, keyCount), Interval: interval, Period: interval.multiply(count)}
	if keyCount > 0 {
		state.NextTie = int(uint64(worker) % uint64(keyCount))
	}
	for index := range state.Slots {
		phase := interval.millis()
		phase.Mul(phase, new(big.Int).SetUint64(uint64(index)))
		phase.Quo(phase, new(big.Int).SetUint64(uint64(max(keyCount, 1))))
		state.Slots[index].Next = epoch.add(interval.multiply(worker)).add(milliseconds(saturatedUint64(phase)))
	}
	return state
}

func refreshSlots(slots []keyState, now scheduleTime) {
	for index := range slots {
		slot := &slots[index]
		if slot.Reset != nil && slot.Reset.compare(now) <= 0 {
			slot.Remaining = nil
			slot.Reset = nil
		}
		if slot.Cooldown != nil && slot.Cooldown.compare(now) <= 0 {
			slot.Cooldown = nil
		}
	}
}
func preferredSlots(available, excluded []int) []int {
	preferred := []int{}
	for _, index := range available {
		if !slices.Contains(excluded, index) {
			preferred = append(preferred, index)
		}
	}
	if len(preferred) > 0 {
		return preferred
	}
	return available
}
func earliestSlots(slots []keyState, candidates []int, now, period scheduleTime) ([]int, scheduleTime) {
	earliest := []int{}
	var start scheduleTime
	for _, index := range candidates {
		aligned := phaseAtOrAfter(slots[index].Next, now, period)
		if len(earliest) == 0 || aligned.compare(start) < 0 {
			earliest = []int{index}
			start = aligned
		} else if aligned == start {
			earliest = append(earliest, index)
		}
	}
	return earliest, start
}
func tieDistance(index, next, count int) int { return (index + count - next) % count }
func extendCooldown(slot *keyState, until scheduleTime) {
	if slot.Cooldown == nil || slot.Cooldown.compare(until) < 0 {
		slot.Cooldown = &until
	}
}

func (state *semanticScheduler) reserve(now scheduleTime, excluded []int) decision {
	refreshSlots(state.Slots, now)
	available := []int{}
	for index, slot := range state.Slots {
		if !slot.IsDisabled && slot.Cooldown == nil {
			available = append(available, index)
		}
	}
	earliest, start := earliestSlots(state.Slots, preferredSlots(available, excluded), now, state.Period)
	if len(earliest) == 0 {
		var until *scheduleTime
		for _, slot := range state.Slots {
			if !slot.IsDisabled && slot.Cooldown != nil && (until == nil || slot.Cooldown.compare(*until) < 0) {
				until = slot.Cooldown
			}
		}
		if until != nil {
			return decision{Kind: "WaitUntil", Until: *until}
		}
		return decision{Kind: "Unavailable"}
	}
	selected := earliest[0]
	for _, index := range earliest[1:] {
		if tieDistance(index, state.NextTie, len(state.Slots)) < tieDistance(selected, state.NextTie, len(state.Slots)) {
			selected = index
		}
	}
	state.Slots[selected].Next = start.add(state.Period)
	state.NextTie = (selected + 1) % len(state.Slots)
	return decision{Kind: "Reserved", Reservation: reservation{selected, start}}
}
func (state *semanticScheduler) finish(reserved reservation, now scheduleTime, outcome health, delay scheduleTime) {
	refreshSlots(state.Slots, now)
	if reserved.Slot < 0 || reserved.Slot >= len(state.Slots) {
		return
	}
	slot := &state.Slots[reserved.Slot]
	switch outcome {
	case healthAuthentication:
		slot.IsDisabled = true
		slot.Cooldown = nil
	case healthRateLimited:
		extendCooldown(slot, now.add(later(delay, state.Interval)))
	case healthTransient:
		if delay != (scheduleTime{}) {
			extendCooldown(slot, now.add(delay))
		}
	}
}
func (state *semanticScheduler) obsolete(reserved reservation, now scheduleTime) bool {
	return now.compare(reserved.Start.add(state.Period).add(state.Interval)) >= 0
}

type openAlexScheduler struct {
	Slots                                    []keyState
	NextTie                                  int
	Period                                   scheduleTime
	Capacity                                 uint64
	ProcessCount                             uint32
	MaximumListCredits, MaximumSearchCredits uint64
}

func newOpenAlexScheduler(keyCount int, processId, processCount uint64, epoch scheduleTime, capacity uint64) *openAlexScheduler {
	worker, count := processContext(processId, processCount)
	state := &openAlexScheduler{Slots: make([]keyState, keyCount), Period: milliseconds(40).multiply(count), Capacity: max(capacity, 1), ProcessCount: count, MaximumListCredits: 1, MaximumSearchCredits: 10}
	if keyCount > 0 {
		state.NextTie = int(uint64(worker) % uint64(keyCount))
	}
	for index := range state.Slots {
		state.Slots[index].Next = epoch.add(milliseconds(40).multiply(worker))
	}
	return state
}
func (state *openAlexScheduler) dailyReserve() uint64 {
	return max(saturatingMultiply(state.Capacity, state.MaximumListCredits), saturatingMultiply(uint64(state.ProcessCount), state.MaximumSearchCredits))
}
func (state *openAlexScheduler) reserve(now scheduleTime, excluded []int) decision {
	refreshSlots(state.Slots, now)
	available := []int{}
	reserve := state.dailyReserve()
	for index, slot := range state.Slots {
		if slot.IsDisabled || slot.Cooldown != nil || slot.Remaining == nil && slot.InFlight > 0 || slot.Remaining != nil && *slot.Remaining <= reserve {
			continue
		}
		available = append(available, index)
	}
	earliest, start := earliestSlots(state.Slots, preferredSlots(available, excluded), now, state.Period)
	if len(earliest) > 0 {
		selected := state.bestSlot(earliest)
		slot := &state.Slots[selected]
		slot.InFlight = saturatingIncrement(slot.InFlight)
		slot.Next = start.add(state.Period)
		state.NextTie = (selected + 1) % len(state.Slots)
		return decision{Kind: "Reserved", Reservation: reservation{selected, start}}
	}
	var until *scheduleTime
	isWaiting := false
	for _, slot := range state.Slots {
		if slot.IsDisabled {
			continue
		}
		hasInsufficientQuota := slot.Remaining != nil && *slot.Remaining <= reserve
		if slot.InFlight > 0 && (slot.Remaining == nil || hasInsufficientQuota && slot.Reset == nil) {
			isWaiting = true
		}
		if slot.Remaining == nil && slot.InFlight > 0 {
			continue
		}
		ready := now
		if slot.Cooldown != nil {
			ready = later(ready, *slot.Cooldown)
		}
		if hasInsufficientQuota {
			if slot.Reset == nil {
				continue
			}
			ready = later(ready, *slot.Reset)
		}
		if ready.compare(now) > 0 && (until == nil || ready.compare(*until) < 0) {
			until = &ready
		}
	}
	if until != nil {
		return decision{Kind: "WaitUntil", Until: *until}
	}
	if isWaiting {
		return decision{Kind: "WaitForChange"}
	}
	return decision{Kind: "Unavailable"}
}

var maximumCredit = new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 127), big.NewInt(1))
var minimumCredit = new(big.Int).Neg(new(big.Int).Lsh(big.NewInt(1), 127))

func clampCredit(value *big.Int) {
	if value.Cmp(maximumCredit) > 0 {
		value.Set(maximumCredit)
	}
	if value.Cmp(minimumCredit) < 0 {
		value.Set(minimumCredit)
	}
}
func (state *openAlexScheduler) bestSlot(candidates []int) int {
	total := new(big.Int)
	for _, index := range candidates {
		slot := &state.Slots[index]
		remaining := uint64(10000)
		if slot.Remaining != nil {
			remaining = *slot.Remaining
		}
		weight := new(big.Int).SetUint64(max(remaining/saturatingIncrement(slot.InFlight), 1))
		slot.Credit.Add(&slot.Credit, weight)
		clampCredit(&slot.Credit)
		total.Add(total, weight)
		clampCredit(total)
	}
	selected := candidates[0]
	for _, index := range candidates[1:] {
		order := state.Slots[index].Credit.Cmp(&state.Slots[selected].Credit)
		if order > 0 || order == 0 && tieDistance(index, state.NextTie, len(state.Slots)) < tieDistance(selected, state.NextTie, len(state.Slots)) {
			selected = index
		}
	}
	state.Slots[selected].Credit.Sub(&state.Slots[selected].Credit, total)
	clampCredit(&state.Slots[selected].Credit)
	return selected
}
func (state *openAlexScheduler) finish(reserved reservation, now scheduleTime, headers rateHeaders, outcome health, delay scheduleTime, isSearch bool) {
	refreshSlots(state.Slots, now)
	hasTrustedQuota := outcome == healthSuccess || outcome == healthDailyLimited
	if hasTrustedQuota && headers.CreditsUsed != nil {
		if isSearch {
			state.MaximumSearchCredits = max(state.MaximumSearchCredits, *headers.CreditsUsed)
		} else {
			state.MaximumListCredits = max(state.MaximumListCredits, *headers.CreditsUsed)
		}
	}
	if reserved.Slot < 0 || reserved.Slot >= len(state.Slots) {
		return
	}
	slot := &state.Slots[reserved.Slot]
	if slot.InFlight > 0 {
		slot.InFlight--
	}
	if hasTrustedQuota {
		if headers.Remaining != nil {
			remaining := *headers.Remaining
			if slot.Remaining != nil {
				remaining = min(remaining, *slot.Remaining)
			}
			slot.Remaining = &remaining
		}
		if headers.ResetAfter != nil {
			reset := now.add(*headers.ResetAfter)
			if slot.Reset == nil || slot.Reset.compare(reset) < 0 {
				slot.Reset = &reset
			}
		}
	}
	switch outcome {
	case healthAuthentication:
		slot.IsDisabled = true
		slot.Cooldown = nil
	case healthRateLimited, healthDailyLimited:
		cooldown := delay
		hasDelay := delay != (scheduleTime{})
		if headers.RetryAfter != nil {
			cooldown = later(cooldown, *headers.RetryAfter)
			hasDelay = true
		}
		if outcome == healthDailyLimited && headers.ResetAfter != nil {
			cooldown = later(cooldown, *headers.ResetAfter)
			hasDelay = true
		}
		if !hasDelay {
			cooldown = seconds(1)
		}
		extendCooldown(slot, now.add(cooldown))
		if outcome == healthDailyLimited && headers.Remaining == nil {
			remaining := uint64(0)
			slot.Remaining = &remaining
		}
	case healthTransient:
		if headers.RetryAfter != nil {
			delay = later(delay, *headers.RetryAfter)
		}
		if delay != (scheduleTime{}) {
			extendCooldown(slot, now.add(delay))
		}
	}
}
func (state *openAlexScheduler) eligible(reserved reservation, now scheduleTime) bool {
	refreshSlots(state.Slots, now)
	if now.compare(reserved.Start.add(state.Period).add(milliseconds(40))) >= 0 || reserved.Slot < 0 || reserved.Slot >= len(state.Slots) {
		return false
	}
	slot := &state.Slots[reserved.Slot]
	return !slot.IsDisabled && slot.Cooldown == nil && (slot.Remaining == nil || *slot.Remaining > state.dailyReserve())
}
func (state *openAlexScheduler) cancel(reserved reservation) {
	if reserved.Slot >= 0 && reserved.Slot < len(state.Slots) && state.Slots[reserved.Slot].InFlight > 0 {
		state.Slots[reserved.Slot].InFlight--
	}
}
