// Package cli composes the canonical application commands without legacy dispatch.
package cli

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

type arguments []string

func (args *arguments) take(name string) (*string, error) {
	index := slices.Index(*args, name)
	if index < 0 {
		return nil, nil
	}
	if index+1 >= len(*args) {
		return nil, fmt.Errorf("%s requires a value", name)
	}
	value := (*args)[index+1]
	*args = slices.Delete(*args, index, index+2)
	return &value, nil
}

func (args *arguments) flag(name string) bool {
	index := slices.Index(*args, name)
	if index < 0 {
		return false
	}
	*args = slices.Delete(*args, index, index+1)
	return true
}

func (args *arguments) projectRoot() (string, error) {
	value, err := args.take("--project-root")
	if err != nil {
		return "", err
	}
	if value != nil {
		return *value, nil
	}
	return os.Getwd()
}

func (args *arguments) authDatabase(root string) (string, error) {
	value, err := args.take("--auth-db")
	if err != nil {
		return "", errors.New("--auth-db requires a path")
	}
	if value != nil {
		return *value, nil
	}
	return filepath.Join(root, "data", "auth.sqlite"), nil
}

func hasHelp(values []string) bool {
	return slices.Contains(values, "--help") || slices.Contains(values, "-h")
}

func parentRunID(values *arguments) (string, error) {
	const option = "--litradar-parent-run-id"
	index := slices.Index(*values, option)
	if index < 0 {
		return "", nil
	}
	if index+1 >= len(*values) {
		return "", errors.New("internal parent run id requires a value")
	}
	value := (*values)[index+1]
	*values = slices.Delete(*values, index, index+2)
	if slices.Contains(*values, option) {
		return "", errors.New("internal parent run id must be unique")
	}
	isValid := len(value) > 0 && len(value) <= 128
	for index, character := range []byte(value) {
		if !isParentRunCharacter(character, index) {
			isValid = false
		}
	}
	if !isValid {
		return "", errors.New("invalid internal parent run id")
	}
	return value, nil
}

// isParentRunCharacter preserves ASCII alphanumeric admission and noninitial correlation punctuation.
func isParentRunCharacter(character byte, index int) bool {
	isAlphaNumeric := character >= '0' && character <= '9' || character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z'
	return isAlphaNumeric || index != 0 && strings.ContainsRune("-_.", rune(character))
}
