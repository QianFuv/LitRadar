package api

import "encoding/json"

func mapBody(element bodyType) bodyType { return bodyType{kind: "map", element: &element} }
func (scanner *bodyScanner) dictionary(element bodyType, path string, depth int) (any, error) {
	scanner.position++
	if depth+1 >= 128 {
		return nil, scanner.failure("recursion limit exceeded", false)
	}
	result := map[string]any{}
	count := 0
	for scanner.peek() != '}' {
		if scanner.position == len(scanner.body) {
			return nil, scanner.failure("EOF while parsing an object", false)
		}
		if count > 0 {
			if err := scanner.separator('}'); err != nil {
				return nil, err
			}
		}
		if scanner.peek() != '"' {
			return nil, scanner.failure("key must be a string", true)
		}
		start := scanner.position
		if err := scanner.stringValue(); err != nil {
			return nil, err
		}
		var key string
		_ = json.Unmarshal(scanner.body[start:scanner.position], &key)
		if scanner.peek() != ':' {
			return nil, scanner.failure("expected `:`", true)
		}
		scanner.position++
		childPath := key
		if path != "" {
			childPath = path + "." + key
		}
		value, err := scanner.typed(element, childPath, depth+1)
		if err != nil {
			return nil, err
		}
		result[key] = value
		count++
	}
	scanner.position++
	return result, nil
}
