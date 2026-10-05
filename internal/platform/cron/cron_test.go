package cron

import (
	"reflect"
	"testing"
	"time"
)

func TestCronBoundaryCases(t *testing.T) {
	for _, scenario := range []struct {
		expression string
		instant    string
		want       bool
	}{
		{"5/2 * * * *", "2026-10-01T00:05:00Z", true},
		{"5/2 * * * *", "2026-10-01T00:07:00Z", false},
		{"0 8 * * 1-7/2", "2026-10-04T08:00:00Z", true},
		{"0 8 * * 0-7/2", "2026-10-04T08:00:00Z", true},
		{"0 8 * * 2-7/2", "2026-10-04T08:00:00Z", false},
		{"0 8 1 * mon", "2026-10-05T08:00:00Z", true},
		{"0 8 */2 * mon", "2026-10-12T08:00:00Z", false},
		{"0 8 1,* * mon", "2026-10-12T08:00:00Z", true},
		{"0 8 * JAN MON-FRI", "2026-01-01T08:00:00Z", true},
	} {
		schedule, err := Parse(scenario.expression)
		if err != nil {
			t.Fatal(err)
		}
		instant, err := time.Parse(time.RFC3339, scenario.instant)
		if err != nil {
			t.Fatal(err)
		}
		if got := schedule.Matches(instant); got != scenario.want {
			t.Errorf("%q at %s: %v", scenario.expression, scenario.instant, got)
		}
	}
	for _, invalid := range []string{"* *", "60 * * * *", "* * * * */0", "* * * * 7-1", "* * * JANUARY *", "1,,2 * * * *", "* * * * 1/2/3"} {
		if _, err := Parse(invalid); err == nil {
			t.Errorf("accepted %q", invalid)
		}
	}
}

func TestUtcSlotIdentityAcrossDstAndNegativeEpoch(t *testing.T) {
	schedule, err := Parse("30 1 * * *")
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		from, to string
		want     []int64
	}{
		{"2016-03-27T00:00:00Z", "2016-03-27T04:00:00Z", []int64{}},
		{"2016-10-30T00:00:00Z", "2016-10-30T03:00:00Z", []int64{1477787400, 1477791000}},
	} {
		from, _ := time.Parse(time.RFC3339, scenario.from)
		to, _ := time.Parse(time.RFC3339, scenario.to)
		actual, err := schedule.Slots("Europe/London", float64(from.Unix()), float64(to.Unix()))
		if err != nil || !reflect.DeepEqual(actual, scenario.want) {
			t.Fatalf("DST slots: %v %v", actual, err)
		}
	}
	every, _ := Parse("* * * * *")
	actual, err := every.Slots("UTC", -60.5, 60)
	if err != nil || !reflect.DeepEqual(actual, []int64{-60, 0, 60}) {
		t.Fatalf("negative boundary: %v %v", actual, err)
	}
}
