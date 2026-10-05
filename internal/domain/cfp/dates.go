package cfp

import (
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"

	"golang.org/x/text/unicode/norm"
)

const monthNames = `Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|Jun(?:e)?|Jul(?:y)?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?`
const datePattern = `(?i)(\d{4})\s*[-/.年]\s*(\d{1,2})\s*[-/.月]\s*(\d{1,2})(?:\s*日)?|(\d{1,2})(?:st|nd|rd|th)?\s*,?\s+(?:of\s+)?(` + monthNames + `)\.?\s*,?\s*(\d{4})|\b(` + monthNames + `)\.?\s+(\d{1,2})(?:st|nd|rd|th)?(?:\s*,\s*|\s+)(\d{4})`

var expressions sync.Map

// PatternCaptures applies the CFP parser's Unicode regex contract and returns byte offsets.
func PatternCaptures(pattern, text string) [][]int { return captures(pattern, text) }

// expression preserves Unicode character classes; boundary assertions are checked separately.
func expression(pattern string) *regexp.Regexp {
	if cached, ok := expressions.Load(pattern); ok {
		return cached.(*regexp.Regexp)
	}
	var translated strings.Builder
	isClass := false
	for index := 0; index < len(pattern); index++ {
		character := pattern[index]
		if character == '\\' && index+1 < len(pattern) {
			index++
			var replacement string
			switch pattern[index] {
			case 's':
				replacement = `\t\n\v\f\r\x{85}\p{Z}`
			case 'w':
				replacement = `\p{L}\p{M}\p{Nd}\p{Pc}\x{200c}\x{200d}\x{2160}-\x{2188}`
			case 'd':
				replacement = `\p{Nd}`
			case 'b':
				translated.WriteString(`(?P<boundary>)`)
				continue
			default:
				translated.WriteByte('\\')
				translated.WriteByte(pattern[index])
				continue
			}
			if !isClass {
				translated.WriteByte('[')
			}
			translated.WriteString(replacement)
			if !isClass {
				translated.WriteByte(']')
			}
			continue
		}
		if character == '[' {
			isClass = true
		}
		if character == ']' {
			isClass = false
		}
		translated.WriteByte(character)
	}
	compiled := regexp.MustCompile(translated.String())
	actual, _ := expressions.LoadOrStore(pattern, compiled)
	return actual.(*regexp.Regexp)
}

func isWord(character rune) bool {
	return unicode.IsLetter(character) || unicode.IsMark(character) || unicode.Is(unicode.Nd, character) || unicode.Is(unicode.Pc, character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) || character == '\u200c' || character == '\u200d'
}

// captures removes private assertion groups while keeping original capture numbering.
func captures(pattern, text string) [][]int {
	compiled := expression(pattern)
	result := [][]int{}
	for _, found := range compiled.FindAllStringSubmatchIndex(text, -1) {
		filtered := append([]int{}, found[:2]...)
		isValid := true
		for index, name := range compiled.SubexpNames()[1:] {
			start, end := found[(index+1)*2], found[(index+1)*2+1]
			if name != "boundary" {
				filtered = append(filtered, start, end)
				continue
			}
			if start < 0 {
				continue
			}
			before, after := rune(0), rune(0)
			if start > 0 {
				before, _ = utf8.DecodeLastRuneInString(text[:start])
			}
			if start < len(text) {
				after, _ = utf8.DecodeRuneInString(text[start:])
			}
			if isWord(before) == isWord(after) {
				isValid = false
			}
		}
		if isValid {
			result = append(result, filtered)
		}
	}
	return result
}

func matches(pattern, value string) bool { return len(captures("(?i)"+pattern, value)) != 0 }
func group(text string, found []int, index int) string {
	if found[index*2] < 0 {
		return ""
	}
	return text[found[index*2]:found[index*2+1]]
}
func number(value string) int { result, _ := strconv.Atoi(value); return result }
func monthNumber(value string) int {
	return slices.Index([]string{"jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"}, strings.ToLower(value[:3])) + 1
}
func calendarDate(year, month, day int) string {
	if month < 1 || month > 12 || day < 1 || day > 31 {
		return ""
	}
	date := time.Date(year, time.Month(month), day, 0, 0, 0, 0, time.UTC)
	if date.Year() != year || int(date.Month()) != month || date.Day() != day {
		return ""
	}
	return date.Format("2006-01-02")
}
func hasNumericBoundaries(text string, start, end int) bool {
	return (start == 0 || text[start-1] < '0' || text[start-1] > '9') && (end == len(text) || text[end] < '0' || text[end] > '9')
}

func matchingText(value string) string {
	normalized := norm.NFKC.String(value)
	normalized = expression(`(?i)\^\{(st|nd|rd|th)\}`).ReplaceAllString(normalized, "$1")
	var result strings.Builder
	previous := 0
	for _, found := range captures(`(\d{1,2})[/.](\d{1,2})[/.](\d{4})`, normalized) {
		result.WriteString(normalized[previous:found[0]])
		first, second := number(group(normalized, found, 1)), number(group(normalized, found, 2))
		if !hasNumericBoundaries(normalized, found[0], found[1]) || first <= 12 && second <= 12 {
			result.WriteString(normalized[found[0]:found[1]])
		} else {
			day, month := first, second
			if first <= 12 {
				day, month = second, first
			}
			result.WriteString(group(normalized, found, 3) + "-" + strconv.Itoa(month) + "-" + strconv.Itoa(day))
		}
		previous = found[1]
	}
	result.WriteString(normalized[previous:])
	return result.String()
}

func dateStage(context string, fallback Stage) Stage {
	normalized := matchingText(context)
	if found := captures(datePattern, normalized); len(found) > 0 {
		prefix := normalized[:found[0][0]]
		if matches(`deadline|due|submit|submission|abstract|proposal|paper|manuscript|open|start|revis|decision|notification|registration|publication|publish|截[稿止]|提交|投稿|摘要|开始|开放`, prefix) {
			context = prefix
		}
	}
	rules := []struct {
		pattern string
		stage   Stage
	}{
		{`registration|payment|报名|缴费`, Registration},
		{`revis(?:ed|ion)|resubmission|final manuscript (?:due|submission)|final paper due|accepted papers?|camera.ready|修改稿|修订稿|返修|终稿`, Revision},
		{`decision|notification|review completed|editorial review|review sent|will let.+know|录用|通知|审稿|评审结果`, Decision},
		{`publication|publish|出版|刊出`, Publication},
		{`conference date|colloquium date|会议日期|举办时间`, Event},
		{`open(?:s|ing)?\s*(?:for (?:new )?submissions?|date|:)|submissions?\s+(?:open|start|begin|from)|(?:begin|start)(?:s|ing)?\s*(?:on|date|:)|开始|开放|起始`, Opens},
		{`abstract|摘要`, Abstract}, {`proposal|提案|建议书`, Proposal},
		{`full\s+(?:paper|manuscript|draft)|completed manuscripts?|(?:paper|manuscript) submission|完整论文|全文|论文投稿`, Paper},
		{`submission|submit|deadline|due|close|截[稿止]|投稿|来稿|征稿时间|征文时间|收稿时间|发送.*论文|论文.*发送`, fallback},
	}
	for _, rule := range rules {
		if matches(rule.pattern, context) {
			return rule.stage
		}
	}
	return ""
}

func clauses(value string) []string {
	result := []string{}
	start := 0
	for position, character := range value {
		switch character {
		case '\n', ';', '；':
			if start < position {
				result = append(result, value[start:position])
			}
			start = position + utf8.RuneLen(character)
		case '。', '！', '？':
			end := position + utf8.RuneLen(character)
			result = append(result, value[start:end])
			start = end
		case '.', '!', '?':
			end := position + 1
			remaining := value[end:]
			first, _ := utf8.DecodeRuneInString(remaining)
			trimmed := strings.TrimLeftFunc(remaining, unicode.IsSpace)
			if unicode.IsSpace(first) && len(trimmed) > 0 && trimmed[0] >= 'A' && trimmed[0] <= 'Z' {
				result = append(result, value[start:end])
				start = end + len(remaining) - len(trimmed)
			}
		}
	}
	if start < len(value) {
		result = append(result, value[start:])
	}
	return result
}

// windowDates returns whether a range was recognized separately from its calendar validity.
func windowDates(text, context string, stage Stage) (string, string, bool) {
	if stage != Opens && !stage.IsSubmission() {
		return "", "", false
	}
	patterns := []string{
		`(?i)(` + monthNames + `)\.?\s+(\d{1,2})(?:st|nd|rd|th)?\s*(?:[–—-]|\band\b)\s*(?:(` + monthNames + `)\.?\s+)?(\d{1,2})(?:st|nd|rd|th)?(?:\s*,\s*|\s+)(\d{4})`,
		`(?i)(\d{1,2})\s+(` + monthNames + `)\s*[–—-]\s*(\d{1,2})\s+(` + monthNames + `)\s+(\d{4})`,
		`(?i)(\d{1,2})\s*[–—-]\s*(\d{1,2})\s+(` + monthNames + `)\s+(\d{4})`,
	}
	for index, pattern := range patterns {
		if index > 0 && !matches(`window|period|窗口|期间`, context) {
			continue
		}
		found := captures(pattern, text)
		if len(found) == 0 {
			continue
		}
		capture := found[0]
		if !hasNumericBoundaries(text, capture[0], capture[1]) {
			return "", "", false
		}
		numericGroups := [][]int{{2, 4, 5}, {1, 3, 5}, {1, 2, 4}}
		for _, position := range numericGroups[index] {
			if _, err := strconv.Atoi(group(text, capture, position)); err != nil {
				return "", "", false
			}
		}
		var year, startMonth, startDay, endMonth, endDay int
		switch index {
		case 0:
			if matches(`\band\b`, group(text, capture, 0)) && !matches(`between|window|period|期间|窗口`, context) {
				return "", "", true
			}
			year = number(group(text, capture, 5))
			startMonth = monthNumber(group(text, capture, 1))
			startDay = number(group(text, capture, 2))
			endMonth = startMonth
			if value := group(text, capture, 3); value != "" {
				endMonth = monthNumber(value)
			}
			endDay = number(group(text, capture, 4))
		case 1:
			year = number(group(text, capture, 5))
			startMonth = monthNumber(group(text, capture, 2))
			startDay = number(group(text, capture, 1))
			endMonth = monthNumber(group(text, capture, 4))
			endDay = number(group(text, capture, 3))
		case 2:
			year = number(group(text, capture, 4))
			startMonth = monthNumber(group(text, capture, 3))
			startDay = number(group(text, capture, 1))
			endMonth = startMonth
			endDay = number(group(text, capture, 2))
		}
		startYear := year
		if startMonth > endMonth {
			startYear--
		}
		start, end := calendarDate(startYear, startMonth, startDay), calendarDate(year, endMonth, endDay)
		if start == "" || end == "" || start > end {
			return "", "", true
		}
		return start, end, true
	}
	return "", "", false
}

// ParseDates returns nil for conflicting/underspecified gates and a non-nil empty slice for no dates.
func ParseDates(value string, entryStage Stage) []Date {
	dates := []Date{}
	unresolved := map[Stage]bool{}
	parts := clauses(CleanText(value))
	for index, clause := range parts {
		text := matchingText(clause)
		context := clause
		if dateStage(clause, entryStage) == "" {
			previous := ""
			if index > 0 {
				previous = parts[index-1]
			}
			context = previous + " " + clause
		}
		stage := dateStage(context, entryStage)
		if stage == "" {
			continue
		}
		submissionStage := entryStage
		if stage.IsSubmission() {
			submissionStage = stage
		}
		isOptional := matches(`optional|if desired|not required|not a prerequisite|可选|自愿|非必需`, context)
		if start, end, hasWindow := windowDates(text, context, stage); hasWindow {
			if start == "" {
				return nil
			}
			dates = append(dates, Date{Date: start, Stage: Opens, OriginalText: clause, IsOptional: isOptional}, Date{Date: end, Stage: submissionStage, OriginalText: clause, IsOptional: isOptional})
			continue
		}
		found := [][]int{}
		for _, capture := range captures(datePattern, text) {
			if hasNumericBoundaries(text, capture[0], capture[1]) {
				found = append(found, capture)
			}
		}
		if len(found) == 0 && (matches(`following.{0,30}(?:timetable|timeline|schedule)|submission (?:stage|information)`, context) || !matches(`deadline|due\b|no later than|\bbefore\b|\bby\b|close|submissions? (?:open|window|period)|截[稿止]|投稿时间|征稿时间|TBD|to be (?:announced|determined)`, context)) {
			continue
		}
		if len(found) == 0 && !isOptional {
			unresolved[stage] = true
		}
		if len(found) == 0 && (stage == Opens || stage.IsSubmission()) && matches(`\d`, clause) {
			return nil
		}
		for matchIndex, capture := range found {
			year, month, day := "", "", ""
			for _, position := range []int{1, 6, 9} {
				if field := group(text, capture, position); field != "" {
					year = field
					break
				}
			}
			for _, position := range []int{3, 4, 8} {
				if field := group(text, capture, position); field != "" {
					day = field
					break
				}
			}
			month = group(text, capture, 2)
			monthValue := number(month)
			if month == "" {
				name := group(text, capture, 5)
				if name == "" {
					name = group(text, capture, 7)
				}
				monthValue = monthNumber(name)
			}
			if _, err := strconv.Atoi(year); err != nil {
				return nil
			}
			if _, err := strconv.Atoi(day); err != nil {
				return nil
			}
			if month != "" {
				if _, err := strconv.Atoi(month); err != nil {
					return nil
				}
			}
			date := calendarDate(number(year), monthValue, number(day))
			if date == "" {
				if stage == Opens || stage.IsSubmission() {
					return nil
				}
				continue
			}
			resolved := stage
			if len(found) == 2 && matches(`window|period|between|窗口|期间`, context) {
				resolved = submissionStage
				if matchIndex == 0 {
					resolved = Opens
				}
			}
			dates = append(dates, Date{Date: date, Stage: resolved, OriginalText: clause, IsOptional: isOptional, IsExclusive: matches(`日前|日之前`, clause) || matches(`\bbefore(?:\s+the)?(?:\s+\w+day,?)?\s*$`, text[:capture[0]])})
		}
	}
	unique := []Date{}
	for _, date := range dates {
		index := slices.IndexFunc(unique, func(existing Date) bool { return existing.Stage == date.Stage && existing.Date == date.Date })
		if index < 0 {
			unique = append(unique, date)
		} else {
			unique[index] = date
		}
	}
	for _, stage := range []Stage{Opens, Abstract, Proposal} {
		if unresolved[stage] && !slices.ContainsFunc(unique, func(date Date) bool { return date.Stage == stage }) && slices.ContainsFunc(unique, func(date Date) bool { return date.Stage.IsSubmission() }) {
			return nil
		}
	}
	for _, stage := range []Stage{Opens, Paper, Abstract, Proposal} {
		count := 0
		extensions := []Date{}
		for _, date := range unique {
			if date.Stage == stage && !date.IsOptional {
				count++
				if matches(`extended|延期|延长`, date.OriginalText) {
					extensions = append(extensions, date)
				}
			}
		}
		if count > 1 {
			if len(extensions) != 1 {
				return nil
			}
			unique = slices.DeleteFunc(unique, func(date Date) bool { return date.Stage == stage && date != extensions[0] })
		}
	}
	var start, end *Date
	for _, date := range unique {
		if date.Stage == Opens && start == nil {
			copy := date
			start = &copy
		}
		if date.Stage == entryStage && end == nil {
			copy := date
			end = &copy
		}
	}
	if start != nil && end != nil && start.Date > end.Date {
		return nil
	}
	return unique
}
