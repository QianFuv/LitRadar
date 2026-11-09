package delivery

import (
	"reflect"
	"strings"
	"testing"
)

func TestNotificationValidationKeepsErrorPriorityAndRawRuneLengths(t *testing.T) {
	longSecret := strings.Repeat("密", 4097)
	cases := []struct {
		value NotificationSettingsUpdate
		want  string
	}{
		{NotificationSettingsUpdate{Keywords: []string{strings.Repeat("中", 501)}, Directions: make([]string, 101)}, "directions must contain at most 100 items"},
		{NotificationSettingsUpdate{Keywords: make([]string, 101), Directions: make([]string, 101)}, "keywords must contain at most 100 items"},
		{NotificationSettingsUpdate{Keywords: []string{strings.Repeat("中", 501)}, SelectedDatabases: make([]string, 501)}, "selected_databases must contain at most 500 items"},
		{NotificationSettingsUpdate{Keywords: []string{strings.Repeat("中", 501)}, Directions: []string{strings.Repeat("中", 501)}}, "keyword must be at most 500 characters"},
		{NotificationSettingsUpdate{DeliveryMethod: "\u2003\t"}, "delivery_method must be 1-32 characters"},
		{NotificationSettingsUpdate{DeliveryMethod: strings.Repeat(" ", 32) + "x"}, "delivery_method must be at most 32 characters"},
		{NotificationSettingsUpdate{DeliveryMethod: "folder", AiBaseUrl: strings.Repeat("中", 2049), PushplusToken: SecretUpdate{true, &longSecret}}, "ai_base_url must be at most 2048 characters"},
		{NotificationSettingsUpdate{DeliveryMethod: "folder", PushplusToken: SecretUpdate{true, &longSecret}, AiApiKey: SecretUpdate{true, &longSecret}}, "pushplus_token must be at most 4096 characters"},
		{NotificationSettingsUpdate{DeliveryMethod: "arbitrary", Keywords: []string{strings.Repeat("中", 500)}, PushplusToken: SecretUpdate{false, &longSecret}}, ""},
		{NotificationSettingsUpdate{DeliveryMethod: "folder", Keywords: []string{strings.Repeat("e\u0301", 250)}, PushplusToken: SecretUpdate{true, nil}}, ""},
		{NotificationSettingsUpdate{DeliveryMethod: "folder", Keywords: []string{strings.Repeat(string([]byte{255}), 500)}}, ""},
		{NotificationSettingsUpdate{DeliveryMethod: "folder", Keywords: []string{strings.Repeat("e\u0301", 251)}}, "keyword must be at most 500 characters"},
	}
	for _, test := range cases {
		err := ValidateNotificationSettings(test.value)
		got := ""
		if err != nil {
			got = err.Error()
		}
		if got != test.want {
			t.Fatalf("got %q want %q", got, test.want)
		}
	}
}

func TestNotificationValidationDoesNotNormalizeOrMutateInput(t *testing.T) {
	secret := "  secret  "
	value := NotificationSettingsUpdate{Keywords: []string{" value ", " value "}, Directions: []string{""}, DeliveryMethod: " folder ", PushplusToken: SecretUpdate{true, &secret}}
	keywords := append([]string(nil), value.Keywords...)
	directions := append([]string(nil), value.Directions...)
	if err := ValidateNotificationSettings(value); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(value.Keywords, keywords) || !reflect.DeepEqual(value.Directions, directions) || value.DeliveryMethod != " folder " || secret != "  secret  " {
		t.Fatal("validation mutated or normalized input")
	}
}
