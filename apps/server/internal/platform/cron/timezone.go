package cron

import (
	_ "embed"
	"fmt"
	"strings"
	"time"
)

//go:embed timezones.txt
var timezoneNames string

// Location restricts zone names to the original locked chrono-tz inventory before loading rules.
func Location(name string) (*time.Location, error) {
	if name == "" || strings.ContainsAny(name, "\r\n") || !strings.Contains("\n"+timezoneNames, "\n"+name+"\n") {
		return nil, fmt.Errorf("timezone must be a valid IANA name")
	}
	location, err := time.LoadLocation(name)
	if err != nil {
		return nil, fmt.Errorf("timezone must be a valid IANA name")
	}
	return location, nil
}
