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

// foldSet normalizes the original set plus its Unicode or ASCII case alternatives.
func foldSet(set runeSet, isUnicode bool) runeSet {
	if isUnicode {
		return normalizeSet(unicodeFoldSet(set))
	}
	return normalizeSet(asciiFoldSet(set))
}

// unicodeFoldSet expands membership using the original set and frozen fold tables.
func unicodeFoldSet(set runeSet) runeSet {
	result := slices.Clone(set)
	for character, alternatives := range unicodeTables.Folds {
		if set.contains(character) {
			for _, other := range alternatives {
				result = append(result, other, other)
			}
		}
	}
	return result
}

// asciiFoldSet adds the opposite ASCII letter case without removing original members.
func asciiFoldSet(set runeSet) runeSet {
	result := slices.Clone(set)
	for character := 'A'; character <= 'Z'; character++ {
		if set.contains(character) || set.contains(character+32) {
			result = append(result, character, character, character+32, character+32)
		}
	}
	return result
}

// asciiClass builds the inclusive ASCII letter and digit ranges for a POSIX class.
func asciiClass(name string) runeSet {
	switch name {
	case "alnum":
		return runeSet{'0', '9', 'A', 'Z', 'a', 'z'}
	case "alpha":
		return runeSet{'A', 'Z', 'a', 'z'}
	case "digit":
		return runeSet{'0', '9'}
	case "lower":
		return runeSet{'a', 'z'}
	case "upper":
		return runeSet{'A', 'Z'}
	case "word":
		return runeSet{'0', '9', 'A', 'Z', '_', '_', 'a', 'z'}
	case "xdigit":
		return runeSet{'0', '9', 'A', 'F', 'a', 'f'}
	default:
		return asciiSpacingClass(name)
	}
}

// asciiSpacingClass builds ASCII spacing, control, visible and punctuation ranges.
func asciiSpacingClass(name string) runeSet {
	switch name {
	case "ascii":
		return runeSet{0, 127}
	case "blank":
		return runeSet{'\t', '\t', ' ', ' '}
	case "cntrl":
		return runeSet{0, 31, 127, 127}
	case "graph":
		return runeSet{33, 126}
	case "print":
		return runeSet{32, 126}
	case "punct":
		return runeSet{33, 47, 58, 64, 91, 96, 123, 126}
	case "space":
		return runeSet{'\t', '\r', ' ', ' '}
	}
	return runeSet{}
}
