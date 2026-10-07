// Package ownedpath preserves the stricter path policy of disposable Crossref worksets.
// General application databases do not inherit this ancestry restriction.
package ownedpath

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Validate requires an ordinary directory or singly linked owned file.
func Validate(path string, isDirectory bool) error {
	metadata, err := os.Lstat(path)
	if err != nil {
		return err
	}
	if isLink(metadata) || metadata.IsDir() != isDirectory || !isDirectory && !metadata.Mode().IsRegular() {
		return fmt.Errorf("Crossref workset path is not an ordinary owned file or directory")
	}
	if !isDirectory && hasMultipleLinks(metadata) {
		return fmt.Errorf("Crossref workset hard links are not allowed")
	}
	return nil
}

// Prepare creates missing ancestors one at a time, validating concurrent creations as well.
// It retains the baseline preflight checks; the database open still needs native NOFOLLOW.
func Prepare(root string) (string, error) {
	if err := validateRootComponents(root); err != nil {
		return "", err
	}
	ancestors := []string{}
	for current := filepath.Clean(root); ; current = filepath.Dir(current) {
		ancestors = append(ancestors, current)
		if current == filepath.Dir(current) {
			break
		}
	}
	for index := len(ancestors) - 1; index >= 0; index-- {
		current := ancestors[index]
		if err := prepareAncestor(current); err != nil {
			return "", err
		}
	}
	return filepath.EvalSymlinks(root)
}

// validateRootComponents rejects relative roots and parent traversal before cleaning the path.
func validateRootComponents(root string) error {
	if !filepath.IsAbs(root) {
		return fmt.Errorf("Crossref workset root must be an absolute core-owned path")
	}
	for _, part := range strings.FieldsFunc(root, func(character rune) bool { return character == '/' || os.PathSeparator == '\\' && character == '\\' }) {
		if part == ".." {
			return fmt.Errorf("Crossref workset root must be an absolute core-owned path")
		}
	}
	return nil
}

// prepareAncestor admits an existing or concurrently created directory using the same no-link policy.
func prepareAncestor(current string) error {
	if _, err := os.Lstat(current); os.IsNotExist(err) {
		if err := os.Mkdir(current, 0755); err != nil && !os.IsExist(err) {
			return err
		}
	} else if err != nil {
		return err
	}
	if err := Validate(current, true); err != nil {
		return err
	}
	return nil
}
