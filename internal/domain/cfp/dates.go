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
	compiled := regexp.MustCompile(translatePattern(pattern))
	actual, _ := expressions.LoadOrStore(pattern, compiled)
	return actual.(*regexp.Regexp)
}

// translatePattern expands Unicode classes while preserving assertion capture placeholders.
func translatePattern(pattern string) string {
	var translated strings.Builder
	isClass := false
	for index := 0; index < len(pattern); index++ {
		character := pattern[index]
		if character == '\\' && index+1 < len(pattern) {
			index++
			writePatternEscape(&translated, pattern[index], isClass)
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
	return translated.String()
}

// writePatternEscape keeps class expansions unwrapped inside an existing character class.
func writePatternEscape(translated *strings.Builder, character byte, isClass bool) {
	var replacement string
	switch character {
	case 's':
		replacement = `\t\n\v\f\r\x{85}\p{Z}`
	case 'w':
		replacement = `\p{L}\p{M}\p{Nd}\p{Pc}\x{200c}\x{200d}\x{2160}-\x{2188}`
	case 'd':
		replacement = `\p{Nd}`
	case 'b':
		translated.WriteString(`(?P<boundary>)`)
		return
	default:
		translated.WriteByte('\\')
		translated.WriteByte(character)
		return
	}
	if !isClass {
		translated.WriteByte('[')
	}
	translated.WriteString(replacement)
	if !isClass {
		translated.WriteByte(']')
	}
}

func isWord(character rune) bool {
	return unicode.IsLetter(character) || unicode.IsMark(character) || unicode.Is(unicode.Nd, character) || unicode.Is(unicode.Pc, character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) || character == '\u200c' || character == '\u200d'
}

// captures removes private assertion groups while keeping original capture numbering.
func captures(pattern, text string) [][]int {
	compiled := expression(pattern)
	result := [][]int{}
	for _, found := range compiled.FindAllStringSubmatchIndex(text, -1) {
		filtered, isValid := filterCaptureBoundaries(compiled.SubexpNames()[1:], found, text)
		if isValid {
			result = append(result, filtered)
		}
	}
	return result
}

// filterCaptureBoundaries removes assertion-only groups without shifting public captures.
func filterCaptureBoundaries(names []string, found []int, text string) ([]int, bool) {
	filtered := append([]int{}, found[:2]...)
	isValid := true
	for index, name := range names {
		start, end := found[(index+1)*2], found[(index+1)*2+1]
		if name != "boundary" {
			filtered = append(filtered, start, end)
			continue
		}
		if start >= 0 && !isWordBoundary(text, start) {
			isValid = false
		}
	}
	return filtered, isValid
}

// isWordBoundary compares Unicode word membership on either side of a byte offset.
func isWordBoundary(text string, position int) bool {
	before, after := rune(0), rune(0)
	if position > 0 {
		before, _ = utf8.DecodeLastRuneInString(text[:position])
	}
	if position < len(text) {
		after, _ = utf8.DecodeRuneInString(text[position:])
	}
	return isWord(before) != isWord(after)
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
			if next, isSentence := nextSentence(value, end); isSentence {
				result = append(result, value[start:end])
				start = next
			}
		}
	}
	if start < len(value) {
		result = append(result, value[start:])
	}
	return result
}

// nextSentence recognizes whitespace followed by an ASCII uppercase sentence start.
func nextSentence(value string, end int) (int, bool) {
	remaining := value[end:]
	first, _ := utf8.DecodeRuneInString(remaining)
	trimmed := strings.TrimLeftFunc(remaining, unicode.IsSpace)
	return end + len(remaining) - len(trimmed), unicode.IsSpace(first) && len(trimmed) > 0 && trimmed[0] >= 'A' && trimmed[0] <= 'Z'
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
		return parseWindowCapture(text, context, capture, index)
	}
	return "", "", false
}

// parseWindowCapture distinguishes recognized invalid ranges from unparsed numeric captures.
func parseWindowCapture(text, context string, capture []int, index int) (string, string, bool) {
	numericGroups := [][]int{{2, 4, 5}, {1, 3, 5}, {1, 2, 4}}
	if !hasIntegerGroups(text, capture, numericGroups[index]) {
		return "", "", false
	}
	if index == 0 && matches(`\band\b`, group(text, capture, 0)) && !matches(`between|window|period|期间|窗口`, context) {
		return "", "", true
	}
	year, startMonth, startDay, endMonth, endDay := windowComponents(text, capture, index)
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

// hasIntegerGroups rejects Unicode digits which the regex recognizes but Atoi cannot convert.
func hasIntegerGroups(text string, capture, positions []int) bool {
	for _, position := range positions {
		if _, err := strconv.Atoi(group(text, capture, position)); err != nil {
			return false
		}
	}
	return true
}

// windowComponents converts each supported shorthand layout into calendar components.
func windowComponents(text string, capture []int, index int) (year, startMonth, startDay, endMonth, endDay int) {
	switch index {
	case 0:
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
	return
}

// ParseDates returns nil for conflicting/underspecified gates and a non-nil empty slice for no dates.
func ParseDates(value string, entryStage Stage) []Date {
	dates := []Date{}
	unresolved := map[Stage]bool{}
	parts := clauses(CleanText(value))
	for index := range parts {
		clause := interpretDateClause(parts, index, entryStage)
		if clause.stage == "" {
			continue
		}
		parsed, isUnresolved, isValid := clause.parse()
		if !isValid {
			return nil
		}
		if isUnresolved {
			unresolved[clause.stage] = true
		}
		dates = append(dates, parsed...)
	}
	return reconcileDates(dates, unresolved, entryStage)
}

// dateClause separates original metadata from normalized matching text and inherited context.
type dateClause struct {
	original        string
	text            string
	context         string
	stage           Stage
	submissionStage Stage
	isOptional      bool
}

// interpretDateClause consults only the immediately preceding clause when a stage is absent.
func interpretDateClause(parts []string, index int, entryStage Stage) dateClause {
	clause := parts[index]
	context := clause
	if dateStage(clause, entryStage) == "" {
		previous := ""
		if index > 0 {
			previous = parts[index-1]
		}
		context = previous + " " + clause
	}
	stage := dateStage(context, entryStage)
	submissionStage := entryStage
	if stage.IsSubmission() {
		submissionStage = stage
	}
	return dateClause{original: clause, text: matchingText(clause), context: context, stage: stage, submissionStage: submissionStage, isOptional: matches(`optional|if desired|not required|not a prerequisite|可选|自愿|非必需`, context)}
}

// parse retains invalid-window precedence and distinguishes unresolved labels from numeric gates.
func (clause dateClause) parse() ([]Date, bool, bool) {
	if start, end, hasWindow := windowDates(clause.text, clause.context, clause.stage); hasWindow {
		if start == "" {
			return nil, false, false
		}
		return []Date{{Date: start, Stage: Opens, OriginalText: clause.original, IsOptional: clause.isOptional}, {Date: end, Stage: clause.submissionStage, OriginalText: clause.original, IsOptional: clause.isOptional}}, false, true
	}
	found := boundedDateCaptures(clause.text)
	if len(found) == 0 {
		if clause.isNonDateLabel() {
			return nil, false, true
		}
		if isCalendarGate(clause.stage) && matches(`\d`, clause.original) {
			return nil, false, false
		}
		return nil, !clause.isOptional, true
	}
	dates, isValid := clause.parseCaptures(found)
	return dates, false, isValid
}

// boundedDateCaptures rejects full-date substrings embedded in longer numeric tokens.
func boundedDateCaptures(text string) [][]int {
	found := [][]int{}
	for _, capture := range captures(datePattern, text) {
		if hasNumericBoundaries(text, capture[0], capture[1]) {
			found = append(found, capture)
		}
	}
	return found
}

// isNonDateLabel excludes timetable introductions and context without a gate announcement.
func (clause dateClause) isNonDateLabel() bool {
	return matches(`following.{0,30}(?:timetable|timeline|schedule)|submission (?:stage|information)`, clause.context) || !matches(`deadline|due\b|no later than|\bbefore\b|\bby\b|close|submissions? (?:open|window|period)|截[稿止]|投稿时间|征稿时间|TBD|to be (?:announced|determined)`, clause.context)
}

// isCalendarGate identifies dates whose invalid calendar values prevent admission.
func isCalendarGate(stage Stage) bool {
	return stage == Opens || stage.IsSubmission()
}

// parseCaptures keeps invalid editorial dates skippable while rejecting invalid gate dates.
func (clause dateClause) parseCaptures(found [][]int) ([]Date, bool) {
	dates := []Date{}
	for index, capture := range found {
		date, hasValidNumbers := capturedCalendarDate(clause.text, capture)
		if !hasValidNumbers {
			return nil, false
		}
		if date == "" {
			if isCalendarGate(clause.stage) {
				return nil, false
			}
			continue
		}
		dates = append(dates, Date{Date: date, Stage: clause.captureStage(index, len(found)), OriginalText: clause.original, IsOptional: clause.isOptional, IsExclusive: matches(`日前|日之前`, clause.original) || matches(`\bbefore(?:\s+the)?(?:\s+\w+day,?)?\s*$`, clause.text[:capture[0]])})
	}
	return dates, true
}

// captureStage treats exactly two fully specified window dates as opening and admission.
func (clause dateClause) captureStage(index, count int) Stage {
	if count == 2 && matches(`window|period|between|窗口|期间`, clause.context) {
		if index == 0 {
			return Opens
		}
		return clause.submissionStage
	}
	return clause.stage
}

// firstCapturedGroup selects the first populated alternative without changing capture numbering.
func firstCapturedGroup(text string, capture, positions []int) string {
	for _, position := range positions {
		if field := group(text, capture, position); field != "" {
			return field
		}
	}
	return ""
}

// capturedCalendarDate separates numeric conversion failures from invalid calendar dates.
func capturedCalendarDate(text string, capture []int) (string, bool) {
	year := firstCapturedGroup(text, capture, []int{1, 6, 9})
	day := firstCapturedGroup(text, capture, []int{3, 4, 8})
	month := group(text, capture, 2)
	monthValue := number(month)
	if month == "" {
		monthValue = monthNumber(firstCapturedGroup(text, capture, []int{5, 7}))
	}
	if _, err := strconv.Atoi(year); err != nil {
		return "", false
	}
	if _, err := strconv.Atoi(day); err != nil {
		return "", false
	}
	if month != "" {
		if _, err := strconv.Atoi(month); err != nil {
			return "", false
		}
	}
	return calendarDate(number(year), monthValue, number(day)), true
}

// reconcileDates preserves deduplication, unresolved-gate, extension and chronology ordering.
func reconcileDates(dates []Date, unresolved map[Stage]bool, entryStage Stage) []Date {
	unique := uniqueDates(dates)
	if hasUnresolvedGate(unique, unresolved) {
		return nil
	}
	for _, stage := range []Stage{Opens, Paper, Abstract, Proposal} {
		var isValid bool
		unique, isValid = resolveDateExtensions(unique, stage)
		if !isValid {
			return nil
		}
	}
	start, end := firstStageDate(unique, Opens), firstStageDate(unique, entryStage)
	if start != nil && end != nil && start.Date > end.Date {
		return nil
	}
	return unique
}

// uniqueDates retains first-position ordering and replaces duplicate metadata with the last clause.
func uniqueDates(dates []Date) []Date {
	unique := []Date{}
	for _, date := range dates {
		index := slices.IndexFunc(unique, func(existing Date) bool { return existing.Stage == date.Stage && existing.Date == date.Date })
		if index < 0 {
			unique = append(unique, date)
		} else {
			unique[index] = date
		}
	}
	return unique
}

// hasUnresolvedGate rejects missing opening/abstract/proposal gates only beside a submission.
func hasUnresolvedGate(dates []Date, unresolved map[Stage]bool) bool {
	for _, stage := range []Stage{Opens, Abstract, Proposal} {
		if unresolved[stage] && !slices.ContainsFunc(dates, func(date Date) bool { return date.Stage == stage }) && slices.ContainsFunc(dates, func(date Date) bool { return date.Stage.IsSubmission() }) {
			return true
		}
	}
	return false
}

// resolveDateExtensions requires exactly one mandatory extension to replace competing dates.
func resolveDateExtensions(dates []Date, stage Stage) ([]Date, bool) {
	count := 0
	extensions := []Date{}
	for _, date := range dates {
		if date.Stage == stage && !date.IsOptional {
			count++
			if matches(`extended|延期|延长`, date.OriginalText) {
				extensions = append(extensions, date)
			}
		}
	}
	if count <= 1 {
		return dates, true
	}
	if len(extensions) != 1 {
		return nil, false
	}
	return slices.DeleteFunc(dates, func(date Date) bool { return date.Stage == stage && date != extensions[0] }), true
}
