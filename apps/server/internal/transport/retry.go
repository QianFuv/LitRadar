package transport

import (
	"math"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode"
)

// Delay retains the full unsigned server delay without time.Duration overflow.
type Delay struct {
	Seconds     uint64
	Nanoseconds uint32
}

// Duration saturates delays beyond Go's timer range; those cannot fit a request budget.
func (delay Delay) Duration() time.Duration {
	if delay.Seconds > uint64(math.MaxInt64)/1_000_000_000 || delay.Seconds == uint64(math.MaxInt64)/1_000_000_000 && uint64(delay.Nanoseconds) > uint64(math.MaxInt64)%1_000_000_000 {
		return time.Duration(math.MaxInt64)
	}
	return time.Duration(delay.Seconds)*time.Second + time.Duration(delay.Nanoseconds)
}

// RetryAfter reads the first visible-ASCII Retry-After header, as reqwest does.
func RetryAfter(headers http.Header) (Delay, bool) {
	value := headers.Get("Retry-After")
	for _, character := range []byte(value) {
		if character != '\t' && (character < 0x20 || character > 0x7e) {
			return Delay{}, false
		}
	}
	return ParseRetryAfter(value, time.Now())
}

var mailDate = regexp.MustCompile(`^(?:([A-Za-z]{3}), *)?([0-9]{1,2}) +([A-Za-z]{3}) +([0-9]{2,}) +([0-9]{2}) *: *([0-9]{2})(?: *:([0-9]{2}))? +([A-Za-z]+|[+-][0-9]{4})(.*)$`)
var obsoleteCalendar = regexp.MustCompile(`^([0-9]{1,2})-([A-Za-z]{3})-([0-9]{2})$`)
var obsoleteClock = regexp.MustCompile(`^ *([0-9]{1,2}): *([0-9]{1,2}): *([0-9]{1,2}) *GMT$`)
var asciiDate = regexp.MustCompile(`^([A-Za-z]{3}) *([A-Za-z]{3}) *([0-9]{1,2}) *([0-9]{1,2}): *([0-9]{1,2}): *([0-9]{1,2}) *([0-9]{1,4}|[+-][0-9]+)$`)

// ParseRetryAfter preserves Chrono's date grammar, fifty-year rule and leap-second subtraction.
func ParseRetryAfter(value string, now time.Time) (Delay, bool) {
	value = strings.TrimSpace(value)
	if value != "" && strings.IndexFunc(value, func(character rune) bool { return character < '0' || character > '9' }) < 0 {
		seconds, err := strconv.ParseUint(value, 10, 64)
		return Delay{Seconds: seconds}, err == nil
	}
	now = now.UTC()
	date, isLeap, isValid := parseMailDate(value)
	if !isValid {
		date, isLeap, isValid = parseObsoleteDate(value, now)
	}
	if !isValid {
		date, isLeap, isValid = parseAsciiDate(value)
	}
	if !isValid {
		return Delay{}, false
	}
	return retryDateDelay(date, isLeap, now), true
}

func secondsOfDay(value time.Time) int {
	return value.Hour()*3600 + value.Minute()*60 + value.Second()
}

func normalizeDateWhitespace(value string) string {
	return strings.Map(func(character rune) rune {
		if unicode.IsSpace(character) {
			return ' '
		}
		return character
	}, value)
}

func parseMailDate(value string) (time.Time, bool, bool) {
	match := mailDate.FindStringSubmatch(normalizeDateWhitespace(value))
	if match == nil || !validDateComments(match[9]) {
		return time.Time{}, false, false
	}
	year, err := strconv.Atoi(match[4])
	if err != nil {
		return time.Time{}, false, false
	}
	switch len(match[4]) {
	case 2:
		if year < 50 {
			year += 2000
		} else {
			year += 1900
		}
	case 3:
		year += 1900
	}
	offset, isValid := dateZone(match[8])
	if !isValid {
		return time.Time{}, false, false
	}
	return buildDate(year, match[3], match[2], match[5], match[6], match[7], match[1], offset)
}

// validDateComments admits only whitespace-separated balanced comments.
func validDateComments(value string) bool {
	for {
		value = strings.TrimSpace(value)
		if value == "" {
			return true
		}
		if value[0] != '(' {
			return false
		}
		index, ok := dateCommentEnd(value)
		if !ok {
			return false
		}
		value = value[index:]
	}
}

// dateZone preserves numeric, military and named zone admission order.
func dateZone(value string) (int, bool) {
	value = strings.ToUpper(value)
	if len(value) == 5 && (value[0] == '+' || value[0] == '-') {
		return numericDateZone(value)
	}
	if len(value) == 1 && value[0] >= 'A' && value[0] <= 'Z' && value != "J" {
		return 0, true
	}
	return namedDateZone(value)
}

func parseObsoleteDate(value string, now time.Time) (time.Time, bool, bool) {
	weekday, remainder, hasWeekday := strings.Cut(value, ", ")
	calendar, clock, hasClock := strings.Cut(remainder, " ")
	match := obsoleteCalendar.FindStringSubmatch(calendar)
	clockMatch := obsoleteClock.FindStringSubmatch(normalizeDateWhitespace(clock))
	if !hasWeekday || !hasClock || match == nil || clockMatch == nil {
		return time.Time{}, false, false
	}
	shortYear, _ := strconv.Atoi(match[3])
	century := now.Year() / 100
	if now.Year() < 0 && now.Year()%100 != 0 {
		century--
	}
	year := century*100 + shortYear
	date, isLeap, isValid := buildDate(year, match[2], match[1], clockMatch[1], clockMatch[2], clockMatch[3], "", 0)
	if !isValid {
		return time.Time{}, false, false
	}
	if beyondFiftyYears(date, isLeap, now) {
		date, isLeap, isValid = buildDate(year-100, match[2], match[1], clockMatch[1], clockMatch[2], clockMatch[3], "", 0)
	}
	return date, isLeap, isValid && date.Weekday().String() == weekday
}

func beyondFiftyYears(date time.Time, isLeap bool, now time.Time) bool {
	nanoseconds := 0
	if isLeap {
		nanoseconds = 1_000_000_000
	}
	actual := []int{date.Year(), int(date.Month()), date.Day(), secondsOfDay(date), nanoseconds}
	threshold := []int{now.Year() + 50, int(now.Month()), now.Day(), secondsOfDay(now), now.Nanosecond()}
	for index, value := range actual {
		if value != threshold[index] {
			return value > threshold[index]
		}
	}
	return false
}

func parseAsciiDate(value string) (time.Time, bool, bool) {
	match := asciiDate.FindStringSubmatch(normalizeDateWhitespace(value))
	if match == nil {
		return time.Time{}, false, false
	}
	year, err := strconv.Atoi(match[7])
	if err != nil {
		return time.Time{}, false, false
	}
	return buildDate(year, match[2], match[3], match[4], match[5], match[6], match[1], 0)
}

// buildDate validates calendar and weekday before applying the zone offset.
func buildDate(year int, monthText, dayText, hourText, minuteText, secondText, weekday string, offset int) (time.Time, bool, bool) {
	month := dateMonth(monthText)
	day, _ := strconv.Atoi(dayText)
	hour, _ := strconv.Atoi(hourText)
	minute, _ := strconv.Atoi(minuteText)
	second, _ := strconv.Atoi(secondText)
	if !validDateComponents(year, month, day, hour, minute, second) {
		return time.Time{}, false, false
	}
	isLeap := second == 60
	date := time.Date(year, time.Month(month), day, hour, minute, min(second, 59), 0, time.UTC)
	if date.Month() != time.Month(month) || date.Day() != day || weekday != "" && !strings.EqualFold(date.Weekday().String()[:3], weekday) {
		return time.Time{}, false, false
	}
	return date.Add(-time.Duration(offset) * time.Second), isLeap, true
}

// retryDateDelay preserves fractional and leap-second subtraction before clamping past dates.
func retryDateDelay(date time.Time, isLeap bool, now time.Time) Delay {
	seconds := date.Unix() - now.Unix()
	nanoseconds := -int64(now.Nanosecond())
	if isLeap {
		nanoseconds += 1_000_000_000
		if secondsOfDay(date) < secondsOfDay(now) {
			seconds--
		}
	}
	if nanoseconds < 0 {
		seconds--
		nanoseconds += 1_000_000_000
	} else if nanoseconds >= 1_000_000_000 {
		seconds++
		nanoseconds -= 1_000_000_000
	}
	if seconds < 0 {
		return Delay{}
	}
	return Delay{Seconds: uint64(seconds), Nanoseconds: uint32(nanoseconds)}
}

// dateCommentEnd preserves nesting and backslash advancement for one parenthesized comment.
func dateCommentEnd(value string) (int, bool) {
	depth, index := 1, 1
	for ; index < len(value) && depth > 0; index++ {
		switch value[index] {
		case '\\':
			index++
		case '(':
			depth++
		case ')':
			depth--
		}
	}
	if depth != 0 || index > len(value) {
		return 0, false
	}
	return index, true
}

// numericDateZone parses the original signed hour/minute slices without additional admission.
func numericDateZone(value string) (int, bool) {
	hours, err := strconv.Atoi(value[1:3])
	minutes, secondErr := strconv.Atoi(value[3:5])
	if err != nil || secondErr != nil || hours >= 24 || minutes >= 60 {
		return 0, false
	}
	seconds := hours*3600 + minutes*60
	if value[0] == '-' {
		seconds = -seconds
	}
	return seconds, true
}

// namedDateZone retains the legacy named zone offsets and rejects unsupported names.
func namedDateZone(value string) (int, bool) {
	switch value {
	case "GMT", "UT":
		return 0, true
	case "EDT":
		return -4 * 3600, true
	case "EST", "CDT":
		return -5 * 3600, true
	case "CST", "MDT":
		return -6 * 3600, true
	case "MST", "PDT":
		return -7 * 3600, true
	case "PST":
		return -8 * 3600, true
	}
	return 0, false
}

// dateMonth resolves the twelve case-insensitive legacy month names.
func dateMonth(monthText string) int {
	month := 0
	for index, name := range []string{"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"} {
		if strings.EqualFold(name, monthText) {
			month = index + 1
			break
		}
	}
	return month
}

// validDateComponents retains Chrono's year bounds and the original component upper limits.
func validDateComponents(year, month, day, hour, minute, second int) bool {
	return !(year < -262143 || year > 262142 || month == 0 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 60)
}
