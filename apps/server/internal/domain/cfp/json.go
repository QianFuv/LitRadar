//go:build go1.27

package cfp

import (
	jsonv2 "encoding/json/v2"
	"errors"
	"reflect"
	"slices"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/jsonobject"
)

var errJSON = errors.New("invalid CFP payload JSON")

func decodeStruct(data []byte, target any, deniesUnknown bool) error {
	if err := jsonobject.DecodeWithOptions(data, target, jsonobject.Options{AllowUnknown: !deniesUnknown}); err != nil {
		return errJSON
	}
	value := reflect.ValueOf(target).Elem()
	for _, field := range value.Fields() {
		if field.Kind() == reflect.Slice && field.IsNil() {
			field.Set(reflect.MakeSlice(field.Type(), 0, 0))
		}
	}
	return nil
}

func encodeStruct(value any) ([]byte, error) { return jsonv2.Marshal(value) }

func decodeEnum(data []byte, allowed []string) (string, error) {
	var value string
	if jsonv2.Unmarshal(data, &value) != nil || !slices.Contains(allowed, value) {
		return "", errJSON
	}
	return value, nil
}

// UnmarshalJSON validates Stage before replacing the receiver.
func (stage *Stage) UnmarshalJSON(data []byte) error {
	decoded, err := decodeEnum(data, []string{"paper", "abstract", "proposal", "opens", "revision", "decision", "publication", "event", "registration"})
	if err == nil {
		*stage = Stage(decoded)
	}
	return err
}

// UnmarshalJSON validates Kind before replacing the receiver.
func (value *Kind) UnmarshalJSON(data []byte) error {
	decoded, err := decodeEnum(data, []string{"special_issue", "general", "proposal", "conference_linked"})
	if err == nil {
		*value = Kind(decoded)
	}
	return err
}

// UnmarshalJSON validates State before replacing the receiver.
func (state *State) UnmarshalJSON(data []byte) error {
	decoded, err := decodeEnum(data, []string{"open", "upcoming", "closed", "historical", "invitation_only", "undated", "uncertain"})
	if err == nil {
		*state = State(decoded)
	}
	return err
}

// UnmarshalJSON validates Source before replacing the receiver.
func (value *Source) UnmarshalJSON(data []byte) error {
	type wire Source
	var decoded wire
	if err := decodeStruct(data, &decoded, true); err != nil {
		return err
	}
	*value = Source(decoded)
	return nil
}

// MarshalJSON encodes Source using the standard JSON representation.
func (value Source) MarshalJSON() ([]byte, error) { type wire Source; return encodeStruct(wire(value)) }

// UnmarshalJSON validates Seed before replacing the receiver.
func (value *Seed) UnmarshalJSON(data []byte) error {
	type wire Seed
	var decoded wire
	if err := decodeStruct(data, &decoded, true); err != nil {
		return err
	}
	*value = Seed(decoded)
	return nil
}

// MarshalJSON encodes Seed using the standard JSON representation.
func (value Seed) MarshalJSON() ([]byte, error) { type wire Seed; return encodeStruct(wire(value)) }

// UnmarshalJSON validates EmptyJournal before replacing the receiver.
func (value *EmptyJournal) UnmarshalJSON(data []byte) error {
	type wire EmptyJournal
	var decoded wire
	if err := decodeStruct(data, &decoded, true); err != nil {
		return err
	}
	*value = EmptyJournal(decoded)
	return nil
}

// MarshalJSON encodes EmptyJournal using the standard JSON representation.
func (value EmptyJournal) MarshalJSON() ([]byte, error) {
	type wire EmptyJournal
	return encodeStruct(wire(value))
}

// UnmarshalJSON validates Notice before replacing the receiver.
func (notice *Notice) UnmarshalJSON(data []byte) error {
	type wire Notice
	var decoded wire
	if err := decodeStruct(data, &decoded, false); err != nil {
		return err
	}
	*notice = Notice(decoded)
	return nil
}

// MarshalJSON encodes Notice using the standard JSON representation.
func (notice Notice) MarshalJSON() ([]byte, error) {
	type wire Notice
	return encodeStruct(wire(notice))
}

// UnmarshalJSON validates Date before replacing the receiver.
func (value *Date) UnmarshalJSON(data []byte) error {
	type wire Date
	var decoded wire
	if err := decodeStruct(data, &decoded, false); err != nil {
		return err
	}
	*value = Date(decoded)
	return nil
}
