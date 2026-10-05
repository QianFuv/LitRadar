package settings

import (
	"testing"
)

func TestRegistryDefaultsAndSecretFlags(t *testing.T) {
	if len(definitions) != 20 {
		t.Fatal("managed setting inventory changed")
	}
	for _, definition := range definitions {
		if normalized, err := Normalize(definition.Field, definition.Default); err != nil || normalized != definition.Default {
			t.Fatalf("%s default is not canonical: %s %v", definition.Field, normalized, err)
		}
	}
}
