package api

import (
	"fmt"
	"unicode/utf8"
)

func validateCharacters(name, value string, maximum int) *apiError {
	if utf8.RuneCountInString(value) > maximum {
		return badRequest(fmt.Sprintf("%s must be at most %d characters", name, maximum))
	}
	return nil
}
func asciiLower(value string) string {
	result := []byte(value)
	for index, character := range result {
		if character >= 'A' && character <= 'Z' {
			result[index] += 'a' - 'A'
		}
	}
	return string(result)
}
