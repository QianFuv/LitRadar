package api

func asciiLower(value string) string {
	result := []byte(value)
	for index, character := range result {
		if character >= 'A' && character <= 'Z' {
			result[index] += 'a' - 'A'
		}
	}
	return string(result)
}
