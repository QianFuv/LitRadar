package settings

import "regexp"

func validLogRegex(pattern string) bool {
	_, err := regexp.Compile(pattern)
	return err == nil
}
