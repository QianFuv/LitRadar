// Package logfilter preserves the original tracing filter and field-matching semantics.
package logfilter

import (
	_ "embed"
	"encoding/json"
	"slices"
)

//go:embed unicode16/tables.json
var unicodeData []byte

var unicodeTables = func() struct {
	Tables map[string]runeSet
	Folds  map[rune][]rune
} {
	var value struct {
		Tables map[string]runeSet
		Folds  map[rune][]rune
	}
	if err := json.Unmarshal(unicodeData, &value); err != nil {
		panic("invalid embedded logging Unicode data")
	}
	return value
}()

type runeSet []rune

func (set runeSet) contains(value rune) bool {
	position, found := slices.BinarySearch(set, value)
	return found || position%2 == 1
}

func normalizeSet(set runeSet) runeSet {
	type interval struct{ first, last rune }
	intervals := make([]interval, 0, len(set)/2)
	for position := 0; position < len(set); position += 2 {
		intervals = append(intervals, interval{set[position], set[position+1]})
	}
	slices.SortFunc(intervals, func(left, right interval) int { return int(left.first - right.first) })
	result := runeSet{}
	for _, item := range intervals {
		if len(result) > 0 && item.first <= result[len(result)-1]+1 {
			result[len(result)-1] = max(result[len(result)-1], item.last)
		} else {
			result = append(result, item.first, item.last)
		}
	}
	return result
}

func unionSet(left, right runeSet) runeSet { return normalizeSet(append(slices.Clone(left), right...)) }

func intersectSet(left, right runeSet) runeSet {
	result := runeSet{}
	for first, second := 0, 0; first < len(left) && second < len(right); {
		start, end := max(left[first], right[second]), min(left[first+1], right[second+1])
		if start <= end {
			result = append(result, start, end)
		}
		if left[first+1] < right[second+1] {
			first += 2
		} else {
			second += 2
		}
	}
	return result
}

func complementSet(set runeSet, isUnicode bool) runeSet {
	maximum := rune(127)
	if isUnicode {
		maximum = 0x10ffff
	}
	result := runeSet{}
	next := rune(0)
	for position := 0; position < len(set); position += 2 {
		if set[position] > next {
			result = append(result, next, min(maximum, set[position]-1))
		}
		next = max(next, set[position+1]+1)
		if next > maximum {
			break
		}
	}
	if next <= maximum {
		result = append(result, next, maximum)
	}
	if isUnicode {
		result = intersectSet(result, runeSet{0, 0xd7ff, 0xe000, 0x10ffff})
	}
	return result
}

func foldSet(set runeSet, isUnicode bool) runeSet {
	result := slices.Clone(set)
	if isUnicode {
		for character, alternatives := range unicodeTables.Folds {
			if set.contains(character) {
				for _, other := range alternatives {
					result = append(result, other, other)
				}
			}
		}
	} else {
		for character := 'A'; character <= 'Z'; character++ {
			if set.contains(character) || set.contains(character+32) {
				result = append(result, character, character, character+32, character+32)
			}
		}
	}
	return normalizeSet(result)
}

func asciiClass(name string) runeSet {
	result := runeSet{}
	for value := rune(0); value < 128; value++ {
		isDigit, isLower, isUpper := value >= '0' && value <= '9', value >= 'a' && value <= 'z', value >= 'A' && value <= 'Z'
		isAlpha := isLower || isUpper
		matches := false
		switch name {
		case "alnum":
			matches = isAlpha || isDigit
		case "alpha":
			matches = isAlpha
		case "ascii":
			matches = true
		case "blank":
			matches = value == ' ' || value == '\t'
		case "cntrl":
			matches = value < 32 || value == 127
		case "digit":
			matches = isDigit
		case "graph":
			matches = value >= 33 && value <= 126
		case "lower":
			matches = isLower
		case "print":
			matches = value >= 32 && value <= 126
		case "punct":
			matches = value >= 33 && value <= 126 && !isAlpha && !isDigit
		case "space":
			matches = value == ' ' || value >= '\t' && value <= '\r'
		case "upper":
			matches = isUpper
		case "word":
			matches = isAlpha || isDigit || value == '_'
		case "xdigit":
			matches = isDigit || value >= 'a' && value <= 'f' || value >= 'A' && value <= 'F'
		}
		if matches {
			result = append(result, value, value)
		}
	}
	return normalizeSet(result)
}
