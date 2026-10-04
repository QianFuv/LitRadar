package scheduler

import (
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/platform/cron"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func TestOriginalRustSchedulerPure(t *testing.T) {
	data, err := os.ReadFile("../../tests/migration/scheduler/worker-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct {
			Input struct {
				Op, Raw, Timezone, Expression, Root, Auth, Executable, Key, Terminal string
				Timeout                                                              uint64
				From, To                                                             float64
				Job                                                                  domain.Job
				Code                                                                 *int
				Bytes                                                                []int
			}
			Expected any
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for index, entry := range corpus.Cases {
		t.Run(fmt.Sprintf("%s-%d", entry.Input.Op, index), func(t *testing.T) {
			input := entry.Input
			var result any
			switch input.Op {
			case "state":
				var state domain.State
				if err := json.Unmarshal([]byte(input.Raw), &state); err != nil {
					result = map[string]any{"error": "json"}
				} else {
					encoded, _ := json.Marshal(state)
					result = map[string]any{"encoded": string(encoded), "terminal": state.IsTerminal()}
				}
			case "zones":
				data, err := os.ReadFile("../platform/cron/timezones.txt")
				if err != nil {
					t.Fatal(err)
				}
				names := strings.Split(strings.TrimSuffix(string(data), "\n"), "\n")
				for _, name := range names {
					if _, err := cron.Location(name); err != nil {
						t.Fatal(name, err)
					}
				}
				result = names
			case "job":
				var job domain.Job
				if err := json.Unmarshal([]byte(input.Raw), &job); err != nil {
					result = map[string]any{"error": "json"}
				} else {
					encoded, err := job.MarshalJSON()
					if err != nil {
						t.Fatal(err)
					}
					var validation any
					if err := job.Validate(); err != nil {
						validation = err.Error()
					}
					result = map[string]any{"encoded": string(encoded), "validation": validation}
				}
			case "timing":
				if err := domain.ValidateTiming(input.Timezone, input.Timeout); err != nil {
					result = err.Error()
				}
			case "cron":
				schedule, err := cron.Parse(input.Expression)
				if err != nil {
					result = map[string]any{"error": err.Error()}
				} else {
					slots, err := schedule.Slots(input.Timezone, input.From, input.To)
					if err != nil {
						result = map[string]any{"error": err.Error()}
					} else {
						result = map[string]any{"slots": slots}
					}
				}
			case "commands":
				commands, err := (ProcessConfig{input.Root, input.Auth, input.Executable, input.Key}).processes(input.Job)
				if err != nil {
					result = map[string]any{"error": err.Error()}
				} else {
					values := []any{}
					for _, command := range commands {
						values = append(values, map[string]any{"command": command.command, "path": command.path, "arguments": command.arguments})
					}
					result = values
				}
			case "summary":
				status, summary := domain.Success, ""
				switch input.Terminal {
				case "cancelled":
					status, summary = domain.Cancelled, "index: cancelled"
				case "timeout":
					status, summary = domain.TimedOut, "index: timed out"
				case "heartbeat":
					status, summary = domain.Unknown, "index: heartbeat lost"
				case "exit":
					status = domain.Failed
					if input.Code == nil {
						summary = "index: process failed"
					} else {
						summary = fmt.Sprintf("index: exit code %d", *input.Code)
					}
				case "supervision":
					status, summary = domain.Error, "index: process supervision failed (spawn_or_assign_failed)"
				}
				output := []byte{}
				for _, value := range input.Bytes {
					output = append(output, byte(value))
				}
				if len(output) > 0 {
					if summary != "" {
						summary += "\n"
					}
					summary += "stdout: " + transport.LossyUtf8(output)
				}
				result = map[string]any{"status": status, "summary": boundedSummary(summary)}
			default:
				t.Fatal("unknown observer operation")
			}
			encoded, err := json.Marshal(result)
			if err != nil {
				t.Fatal(err)
			}
			var actual any
			json.Unmarshal(encoded, &actual)
			if !reflect.DeepEqual(actual, entry.Expected) {
				expected, _ := json.Marshal(entry.Expected)
				t.Fatalf("input %#v: got %s want %s", input, encoded, expected)
			}
		})
	}
}
