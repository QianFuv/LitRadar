package runtime

import (
	"net/http"
	"strconv"
	"strings"
	"time"
	"unicode"
)

func prefersGzip(headers http.Header) bool {
	gzipQuality, identityQuality := 0, 0
	for _, line := range headers.Values("Accept-Encoding") {
		if !isVisibleHeader(line) {
			continue
		}
		for _, item := range strings.Split(line, ",") {
			encoding, weight, hasWeight := strings.Cut(item, ";")
			quality := 1000
			if hasWeight {
				quality = encodingQuality(strings.TrimSpace(weight))
			}
			switch strings.ToLower(strings.TrimSpace(encoding)) {
			case "gzip", "x-gzip":
				gzipQuality = max(gzipQuality, quality)
			case "identity":
				identityQuality = max(identityQuality, quality)
			}
		}
	}
	return gzipQuality > 0 && gzipQuality >= identityQuality
}

func encodingQuality(value string) int {
	if !hasEncodingQualityPrefix(value) {
		return 0
	}
	quality := int(value[2]-'0') * 1000
	if len(value) == 3 {
		return quality
	}
	if value[3] != '.' || len(value) > 7 {
		return 0
	}
	factor := 100
	for _, digit := range value[4:] {
		if digit < '0' || digit > '9' {
			return 0
		}
		quality += int(digit-'0') * factor
		factor /= 10
	}
	if quality > 1000 {
		return 0
	}
	return quality
}

// frontendPrecondition evaluates date validators only when the asset has a known modification time.
func frontendPrecondition(headers http.Header, etag string, modified time.Time) int {
	modified = modified.Truncate(time.Second)
	if failsFrontendMatch(headers, etag, modified) {
		return http.StatusPreconditionFailed
	}
	if match := headers.Get("If-None-Match"); match != "" {
		if etag != "" && matchesEtag(match, etag, true) {
			return http.StatusNotModified
		}
	} else if date, isValid := frontendDate(headers.Get("If-Modified-Since")); !modified.IsZero() && isValid && !date.Before(modified) {
		return http.StatusNotModified
	}
	return http.StatusOK
}

func isVisibleHeader(value string) bool {
	for _, character := range []byte(value) {
		if character != '\t' && (character < 0x20 || character > 0x7e) {
			return false
		}
	}
	return true
}

func frontendDate(value string) (time.Time, bool) {
	if !isAsciiDate(value) {
		return time.Time{}, false
	}
	value = strings.TrimSpace(value)
	for _, layout := range []string{http.TimeFormat, time.RFC850, time.ANSIC} {
		candidate, isValid := frontendDateCandidate(value, layout)
		if !isValid {
			continue
		}
		date, err := time.Parse(layout, candidate)
		if err != nil {
			continue
		}
		if layout == time.RFC850 && date.Year() == 1969 {
			date = date.AddDate(100, 0, 0)
		}
		if date.Year() >= 1970 && date.Year() <= 9999 && date.Format(layout) == candidate {
			return date, true
		}
	}
	return time.Time{}, false
}

func matchesEtag(value, etag string, isWeak bool) bool {
	if value == "*" {
		return true
	}
	start, isQuoted := 0, false
	for index := 0; index <= len(value); index++ {
		if index < len(value) && value[index] == '"' {
			isQuoted = !isQuoted
		}
		if index == len(value) || value[index] == ',' && !isQuoted {
			candidate := strings.Trim(value[start:index], " \t")
			if isWeak {
				candidate = strings.TrimPrefix(candidate, "W/")
			}
			if candidate == etag {
				return true
			}
			start = index + 1
		}
	}
	return false
}

func frontendRanges(value string, size uint64) ([][2]uint64, bool) {
	if !strings.HasPrefix(value, "bytes=") {
		return nil, false
	}
	value = strings.TrimPrefix(value, "bytes=")
	if len(value) == 0 || unicode.IsSpace([]rune(value)[0]) {
		return nil, false
	}
	ranges := [][2]uint64{}
	last := uint64(0)
	if size > 0 {
		last = size - 1
	}
	for _, item := range strings.Split(value, ",") {
		current, isValid := frontendRangeItem(item, size, last)
		if !isValid || overlapsFrontendRange(current, ranges) {
			return nil, false
		}
		ranges = append(ranges, current)
	}
	return ranges, true
}

func strictRangeNumber(value string) (uint64, bool) {
	if value == "" || value[0] == '+' || len(value) > 1 && value[0] == '0' {
		return 0, false
	}
	number, err := strconv.ParseUint(value, 10, 64)
	return number, err == nil
}

// hasEncodingQualityPrefix checks the fixed admission grammar before fractional parsing.
func hasEncodingQualityPrefix(value string) bool {
	return !(len(value) < 3 || (value[0] != 'q' && value[0] != 'Q') || value[1] != '=' || (value[2] != '0' && value[2] != '1'))
}

// failsFrontendMatch retains If-Match precedence over the unmodified-since date.
func failsFrontendMatch(headers http.Header, etag string, modified time.Time) bool {
	if match := headers.Get("If-Match"); match != "" {
		if etag == "" || !matchesEtag(match, etag, false) {
			return true
		}
	} else if date, isValid := frontendDate(headers.Get("If-Unmodified-Since")); !modified.IsZero() && isValid && date.Before(modified) {
		return true
	}
	return false
}

// isAsciiDate rejects non-ASCII bytes before trimming and parsing HTTP dates.
func isAsciiDate(value string) bool {
	for _, character := range []byte(value) {
		if character > 127 {
			return false
		}
	}
	return true
}

// frontendDateCandidate retains the original RFC850 and ANSIC spelling adjustments.
func frontendDateCandidate(value, layout string) (string, bool) {
	candidate := value
	if layout == time.RFC850 {
		_, suffix, exists := strings.Cut(candidate, ", ")
		if !exists || len(suffix) != 22 {
			return "", false
		}
		offset := len(candidate) - len(suffix) + 9
		candidate = candidate[:offset] + " " + candidate[offset+1:]
	}
	if layout == time.ANSIC && len(candidate) == 24 && candidate[8] == '0' {
		candidate = candidate[:8] + " " + candidate[9:]
	}
	return candidate, true
}

// frontendRangeItem checks whitespace, separators and ordered bounds of one source range.
func frontendRangeItem(item string, size, last uint64) ([2]uint64, bool) {
	whitespace := 0
	for _, character := range item {
		if unicode.IsSpace(character) {
			whitespace++
		}
	}
	if whitespace > 1 || len(item) == 0 || strings.TrimRightFunc(item, unicode.IsSpace) != item {
		return [2]uint64{}, false
	}
	first, final, exists := strings.Cut(strings.TrimSpace(item), "-")
	if !exists {
		return [2]uint64{}, false
	}
	start, end, isValid := frontendRangeBounds(first, final, size, last)
	if !isValid {
		return [2]uint64{}, false
	}
	if start > end {
		return [2]uint64{}, false
	}
	return [2]uint64{start, end}, true
}

// frontendRangeBounds preserves suffix rejection and explicit-end clamping.
func frontendRangeBounds(first, final string, size, last uint64) (uint64, uint64, bool) {
	start, end := uint64(0), last
	if first == "" {
		suffix, isValid := strictRangeNumber(final)
		if !isValid || suffix == 0 || suffix > size {
			return 0, 0, false
		}
		start = size - suffix
	} else {
		var isValid bool
		start, isValid = strictRangeNumber(first)
		if !isValid {
			return 0, 0, false
		}
		if final != "" {
			end, isValid = strictRangeNumber(final)
			if !isValid {
				return 0, 0, false
			}
			end = min(end, last)
		}
	}
	return start, end, true
}

// overlapsFrontendRange rejects intersecting intervals while retaining adjacent source order.
func overlapsFrontendRange(current [2]uint64, ranges [][2]uint64) bool {
	for _, previous := range ranges {
		if current[0] <= previous[1] && current[1] >= previous[0] {
			return true
		}
	}
	return false
}
