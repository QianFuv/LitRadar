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
	if len(value) < 3 || (value[0] != 'q' && value[0] != 'Q') || value[1] != '=' || (value[2] != '0' && value[2] != '1') {
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

func frontendPrecondition(headers http.Header, etag string, modified time.Time) int {
	modified = modified.Truncate(time.Second)
	if match := headers.Get("If-Match"); match != "" {
		if etag == "" || !matchesEtag(match, etag, false) {
			return http.StatusPreconditionFailed
		}
	} else if date, isValid := frontendDate(headers.Get("If-Unmodified-Since")); isValid && date.Before(modified) {
		return http.StatusPreconditionFailed
	}
	if match := headers.Get("If-None-Match"); match != "" {
		if etag != "" && matchesEtag(match, etag, true) {
			return http.StatusNotModified
		}
	} else if date, isValid := frontendDate(headers.Get("If-Modified-Since")); isValid && !date.Before(modified) {
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
	for _, character := range []byte(value) {
		if character > 127 {
			return time.Time{}, false
		}
	}
	value = strings.TrimSpace(value)
	for _, layout := range []string{http.TimeFormat, time.RFC850, time.ANSIC} {
		candidate := value
		if layout == time.RFC850 {
			_, suffix, exists := strings.Cut(candidate, ", ")
			if !exists || len(suffix) != 22 {
				continue
			}
			offset := len(candidate) - len(suffix) + 9
			candidate = candidate[:offset] + " " + candidate[offset+1:]
		}
		if layout == time.ANSIC && len(candidate) == 24 && candidate[8] == '0' {
			candidate = candidate[:8] + " " + candidate[9:]
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
		whitespace := 0
		for _, character := range item {
			if unicode.IsSpace(character) {
				whitespace++
			}
		}
		if whitespace > 1 || len(item) == 0 || strings.TrimRightFunc(item, unicode.IsSpace) != item {
			return nil, false
		}
		first, final, exists := strings.Cut(strings.TrimSpace(item), "-")
		if !exists {
			return nil, false
		}
		start, end := uint64(0), last
		if first == "" {
			suffix, isValid := strictRangeNumber(final)
			if !isValid || suffix == 0 || suffix > size {
				return nil, false
			}
			start = size - suffix
		} else {
			var isValid bool
			start, isValid = strictRangeNumber(first)
			if !isValid {
				return nil, false
			}
			if final != "" {
				end, isValid = strictRangeNumber(final)
				if !isValid {
					return nil, false
				}
				end = min(end, last)
			}
		}
		if start > end {
			return nil, false
		}
		for _, previous := range ranges {
			if start <= previous[1] && end >= previous[0] {
				return nil, false
			}
		}
		ranges = append(ranges, [2]uint64{start, end})
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
