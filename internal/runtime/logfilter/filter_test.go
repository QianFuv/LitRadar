package logfilter

import (
	"encoding/json"
	"math"
	"os"
	"reflect"
	"strconv"
	"testing"
)

type observedEvent struct {
	Phase int     `json:"phase"`
	Level string  `json:"level"`
	Span  *string `json:"span"`
}
type typedValue struct {
	Kind  string
	Value any
}

func (value typedValue) decoded() any {
	switch value.Kind {
	case "bool", "str":
		return value.Value
	case "debug":
		return DebugValue(strconv.Quote(value.Value.(string)))
	case "u64":
		number, _ := strconv.ParseUint(value.Value.(string), 10, 64)
		return number
	case "i64":
		number, _ := strconv.ParseInt(value.Value.(string), 10, 64)
		return number
	case "f64":
		if value.Value == "NaN" {
			return math.NaN()
		}
		number, _ := strconv.ParseFloat(value.Value.(string), 64)
		return number
	}
	return nil
}

func TestFilterMatchesOriginalSubscriberScopes(t *testing.T) {
	data, err := os.ReadFile("../../../tests/migration/runtime/log-filter-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Cases []struct {
			Filter, Target, Command string
			Initial, Update         *typedValue
			Valid                   bool
			Observations            []observedEvent
		}
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	for index, item := range corpus.Cases {
		t.Run(strconv.Itoa(index)+"_"+item.Filter, func(t *testing.T) {
			filter, err := Parse(item.Filter)
			if (err == nil) != item.Valid {
				t.Fatalf("validity changed: %v want=%v", err, item.Valid)
			}
			if err != nil {
				return
			}
			actual := []observedEvent{}
			emit := func(phase int, scope []Entry) {
				for level := Error; level <= Trace; level++ {
					if filter.Enabled(item.Target, level, map[string]any{"event": "probe", "component": "fixture", "phase": phase, "value": "sample"}, scope) {
						var name *string
						if len(scope) > 0 && scope[0].Span != nil {
							value := "process"
							name = &value
						}
						actual = append(actual, observedEvent{phase, []string{"", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"}[level], name})
					}
				}
			}
			emit(0, nil)
			command := item.Command
			if command == "" {
				command = "serve"
			}
			span := filter.NewSpan("litradar", "process", Info, map[string]any{"component": "runtime", "command": command, "value": nil}, nil)
			if item.Initial != nil {
				span.Record("value", item.Initial.decoded())
			}
			scope := []Entry{span.Enter()}
			emit(1, scope)
			if item.Update != nil {
				span.Record("value", item.Update.decoded())
			}
			emit(2, scope)
			emit(3, []Entry{span.Enter()})
			emit(4, nil)
			if !reflect.DeepEqual(actual, item.Observations) {
				first, _ := json.Marshal(actual)
				second, _ := json.Marshal(item.Observations)
				t.Errorf("actual %s\nexpected %s", first, second)
			}
		})
	}
}
