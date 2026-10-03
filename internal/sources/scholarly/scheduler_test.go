package scholarly

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"testing"
)

type frozenDuration struct {
	Seconds     string
	Nanoseconds uint32
}

func (value frozenDuration) duration() scheduleTime {
	seconds, _ := strconv.ParseUint(value.Seconds, 10, 64)
	return scheduleTime{seconds, value.Nanoseconds}
}
func encodedDuration(value scheduleTime) any {
	return map[string]any{"seconds": strconv.FormatUint(value.Seconds, 10), "nanoseconds": value.Nanoseconds}
}
func encodedOptionalDuration(value *scheduleTime) any {
	if value == nil {
		return nil
	}
	return encodedDuration(*value)
}
func encodedOptionalUnsigned(value *uint64) any {
	if value == nil {
		return nil
	}
	return strconv.FormatUint(*value, 10)
}
func optionalUnsigned(value *string) *uint64 {
	if value == nil {
		return nil
	}
	parsed, _ := strconv.ParseUint(*value, 10, 64)
	return &parsed
}
func optionalDuration(value *frozenDuration) *scheduleTime {
	if value == nil {
		return nil
	}
	parsed := value.duration()
	return &parsed
}

func TestFrozenSchedulers(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/sources/scheduler-vectors.json.gz")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures struct {
		Observations []struct {
			Id, Service     string
			Keys            int
			ProcessId       string `json:"process_id"`
			ProcessCount    string `json:"process_count"`
			Capacity        string
			Epoch, Interval frozenDuration
			Operations      []struct {
				Op                 string
				Now, Start, Delay  frozenDuration
				Excluded           []int
				Slot               int
				Health             health
				Remaining, Credits *string
				Reset, Retry       *frozenDuration
				IsSearch           bool `json:"search"`
			}
			Output []any
		}
	}
	compressed, err := gzip.NewReader(bytes.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	defer compressed.Close()
	decoder := json.NewDecoder(compressed)
	decoder.UseNumber()
	if err := decoder.Decode(&fixtures); err != nil {
		t.Fatal(err)
	}
	for _, fixture := range fixtures.Observations {
		t.Run(fixture.Id, func(t *testing.T) {
			processId, _ := strconv.ParseUint(fixture.ProcessId, 10, 64)
			processCount, _ := strconv.ParseUint(fixture.ProcessCount, 10, 64)
			capacity, _ := strconv.ParseUint(fixture.Capacity, 10, 64)
			openAlex := newOpenAlexScheduler(fixture.Keys, processId, processCount, fixture.Epoch.duration(), capacity)
			semantic := newSemanticScheduler(fixture.Keys, processId, processCount, fixture.Epoch.duration(), fixture.Interval.duration())
			isOpenAlex := fixture.Service == OpenAlex
			for index, operation := range fixture.Operations {
				var result any
				reserved := reservation{operation.Slot, operation.Start.duration()}
				now := operation.Now.duration()
				switch operation.Op {
				case "reserve":
					var choice decision
					if isOpenAlex {
						choice = openAlex.reserve(now, operation.Excluded)
					} else {
						choice = semantic.reserve(now, operation.Excluded)
					}
					value := map[string]any{"kind": choice.Kind}
					if choice.Kind == "Reserved" {
						value["slot"] = choice.Reservation.Slot
						value["start"] = encodedDuration(choice.Reservation.Start)
					}
					if choice.Kind == "WaitUntil" {
						value["until"] = encodedDuration(choice.Until)
					}
					result = value
				case "finish":
					if isOpenAlex {
						openAlex.finish(reserved, now, rateHeaders{Remaining: optionalUnsigned(operation.Remaining), CreditsUsed: optionalUnsigned(operation.Credits), ResetAfter: optionalDuration(operation.Reset), RetryAfter: optionalDuration(operation.Retry)}, operation.Health, operation.Delay.duration(), operation.IsSearch)
					} else {
						semantic.finish(reserved, now, operation.Health, operation.Delay.duration())
					}
				case "cancel":
					openAlex.cancel(reserved)
				case "eligible":
					if isOpenAlex {
						result = openAlex.eligible(reserved, now)
					} else {
						result = !semantic.obsolete(reserved, now)
					}
				default:
					t.Fatalf("unknown operation %s", operation.Op)
				}
				slots := []any{}
				state := map[string]any{}
				if isOpenAlex {
					state["next_tie"] = openAlex.NextTie
					state["period"] = encodedDuration(openAlex.Period)
					state["reserve"] = strconv.FormatUint(openAlex.dailyReserve(), 10)
					for _, slot := range openAlex.Slots {
						slots = append(slots, map[string]any{"next": encodedDuration(slot.Next), "cooldown": encodedOptionalDuration(slot.Cooldown), "disabled": slot.IsDisabled, "remaining": encodedOptionalUnsigned(slot.Remaining), "reset": encodedOptionalDuration(slot.Reset), "in_flight": strconv.FormatUint(slot.InFlight, 10), "credit": slot.Credit.String()})
					}
				} else {
					state["next_tie"] = semantic.NextTie
					state["period"] = encodedDuration(semantic.Period)
					for _, slot := range semantic.Slots {
						slots = append(slots, map[string]any{"next": encodedDuration(slot.Next), "cooldown": encodedOptionalDuration(slot.Cooldown), "disabled": slot.IsDisabled})
					}
				}
				state["slots"] = slots
				actual := normalizedTestJson(t, map[string]any{"result": result, "state": state})
				if !reflect.DeepEqual(actual, fixture.Output[index]) {
					encoded, _ := json.Marshal(actual)
					expected, _ := json.Marshal(fixture.Output[index])
					t.Fatalf("step %d %s\ngot %s\nwant %s", index, operation.Op, encoded, expected)
				}
			}
		})
	}
}
