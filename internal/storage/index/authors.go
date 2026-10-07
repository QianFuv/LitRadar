package index

import (
	"encoding/json"
	"fmt"
	"io"
	"strconv"
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// AuthorJsonError preserves the first typed decoding error and its UTF-8 byte location.
type AuthorJsonError struct {
	Message      string
	Line, Column int
}

func (err *AuthorJsonError) Error() string {
	return fmt.Sprintf("%s at line %d column %d", err.Message, err.Line, err.Column)
}

type authorDecoder struct {
	text     string
	position int
}

func (decoder *authorDecoder) failure(message string, position int) error {
	position = min(position, len(decoder.text))
	prefix := decoder.text[:position]
	line := strings.Count(prefix, "\n") + 1
	column := position
	if last := strings.LastIndexByte(prefix, '\n'); last >= 0 {
		column = position - last - 1
	}
	return &AuthorJsonError{message, line, column}
}
func (decoder *authorDecoder) whitespace() {
	for decoder.position < len(decoder.text) && strings.ContainsRune(" \r\n\t", rune(decoder.text[decoder.position])) {
		decoder.position++
	}
}
func (decoder *authorDecoder) token(expected string) (json.Token, error) {
	decoder.whitespace()
	start := decoder.position
	if start == len(decoder.text) {
		return nil, decoder.failure("EOF while parsing a value", start)
	}
	token, err := decoder.readToken(start)
	if err != nil {
		return nil, err
	}
	description, err := decoder.describeToken(token)
	if err != nil {
		return nil, err
	}
	if !acceptsAuthorToken(expected, token) {
		position := decoder.position
		if _, isDelimiter := token.(json.Delim); isDelimiter {
			position = start
		}
		return nil, decoder.failure("invalid type: "+description+", expected "+expected, position)
	}
	return token, nil
}

func (decoder *authorDecoder) readToken(start int) (json.Token, error) {
	if decoder.text[start] == '"' {
		return decoder.stringToken()
	}
	stream := json.NewDecoder(strings.NewReader(decoder.text[start:]))
	stream.UseNumber()
	token, err := stream.Token()
	if err == nil {
		decoder.position += int(stream.InputOffset())
		return token, nil
	}
	if err == io.EOF || err == io.ErrUnexpectedEOF {
		return nil, decoder.failure("EOF while parsing a value", len(decoder.text))
	}
	position := start + 1
	if syntax, ok := err.(*json.SyntaxError); ok {
		position = start + int(syntax.Offset)
	}
	return nil, decoder.failure("expected value", position)
}

func (decoder *authorDecoder) describeToken(token json.Token) (string, error) {
	switch value := token.(type) {
	case nil:
		return "null", nil
	case bool:
		return fmt.Sprintf("boolean `%t`", value), nil
	case json.Number:
		number, err := domain.ParseNumber(value)
		if err != nil {
			return "", decoder.failure("number out of range", decoder.position)
		}
		kind := "integer"
		_, isUnsigned := number.AsUint64()
		_, isSigned := number.AsInt64()
		if !isUnsigned && !isSigned {
			kind = "floating point"
		}
		return kind + " `" + number.String() + "`", nil
	case string:
		return "string " + debugString(value), nil
	case json.Delim:
		if value == '[' {
			return "sequence", nil
		}
		return "map", nil
	}
	return "", nil
}

func acceptsAuthorToken(expected string, token json.Token) bool {
	isAccepted := expected == "a sequence" && token == json.Delim('[') || expected == "struct ArticleAuthorDraft" && (token == json.Delim('[') || token == json.Delim('{'))
	if _, isString := token.(string); expected == "a string" && isString {
		isAccepted = true
	}
	return isAccepted
}

func decodeCanonicalAuthors(text string) ([]domain.ArticleAuthorDraft, error) {
	var authors []domain.ArticleAuthorDraft
	strictError := json.Unmarshal([]byte(text), &authors)
	if strictError == nil && authors != nil {
		return authors, nil
	}
	_, diagnostic := diagnoseCanonicalAuthors(text)
	if diagnostic != nil {
		return nil, diagnostic
	}
	return nil, strictError
}

func diagnoseCanonicalAuthors(text string) ([]domain.ArticleAuthorDraft, error) {
	decoder := authorDecoder{text: text}
	if _, err := decoder.token("a sequence"); err != nil {
		return nil, err
	}
	values := []domain.ArticleAuthorDraft{}
	for {
		decoder.whitespace()
		if decoder.position == len(text) {
			return nil, decoder.failure("EOF while parsing a list", decoder.position)
		}
		if text[decoder.position] == ']' {
			decoder.position++
			break
		}
		value, err := decoder.author()
		if err != nil {
			return nil, err
		}
		values = append(values, value)
		isClosed, err := decoder.finishAuthorElement()
		if err != nil {
			return nil, err
		}
		if isClosed {
			break
		}
	}
	decoder.whitespace()
	if decoder.position != len(text) {
		return nil, decoder.failure("trailing characters", decoder.position+1)
	}
	return values, nil
}

func (decoder *authorDecoder) author() (domain.ArticleAuthorDraft, error) {
	var author domain.ArticleAuthorDraft
	shape, err := decoder.token("struct ArticleAuthorDraft")
	if err != nil {
		return author, err
	}
	if shape == json.Delim('[') {
		return decoder.authorSequence()
	}
	hasName := false
	for {
		decoder.whitespace()
		if decoder.position == len(decoder.text) {
			return author, decoder.failure("EOF while parsing an object", decoder.position)
		}
		isClosed, err := decoder.closeAuthorObject(hasName)
		if isClosed {
			return author, err
		}
		if err := decoder.authorNameField(&author, hasName); err != nil {
			return author, err
		}
		hasName = true
		isClosed, err = decoder.finishAuthorField()
		if err != nil {
			return author, err
		}
		if isClosed {
			return author, nil
		}
	}
}

func debugString(value string) string {
	var output strings.Builder
	output.WriteByte('"')
	for _, character := range value {
		writeAuthorDebugRune(&output, character)
	}
	output.WriteByte('"')
	return output.String()
}

func (decoder *authorDecoder) stringToken() (string, error) {
	start := decoder.position
	decoder.position++
	for decoder.position < len(decoder.text) {
		character := decoder.text[decoder.position]
		decoder.position++
		if character == '"' {
			var value string
			if err := json.Unmarshal([]byte(decoder.text[start:decoder.position]), &value); err != nil {
				return "", err
			}
			return value, nil
		}
		if character < 32 {
			return "", decoder.failure("control character (\\u0000-\\u001F) found while parsing a string", decoder.position)
		}
		if character != '\\' {
			continue
		}
		if decoder.position == len(decoder.text) {
			break
		}
		if err := decoder.stringEscape(); err != nil {
			return "", err
		}
	}
	return "", decoder.failure("EOF while parsing a string", decoder.position)
}

func (decoder *authorDecoder) hexEscape() (uint64, error) {
	if len(decoder.text)-decoder.position < 4 {
		decoder.position = len(decoder.text)
		return 0, decoder.failure("EOF while parsing a string", decoder.position)
	}
	value, err := strconv.ParseUint(decoder.text[decoder.position:decoder.position+4], 16, 16)
	if err != nil {
		for position := 0; position < 4; position++ {
			character := decoder.text[decoder.position]
			decoder.position++
			if !strings.ContainsRune("0123456789abcdefABCDEF", rune(character)) {
				break
			}
		}
		return 0, decoder.failure("invalid escape", decoder.position)
	}
	decoder.position += 4
	return value, nil
}

func (decoder *authorDecoder) finishAuthorElement() (bool, error) {
	decoder.whitespace()
	if decoder.position == len(decoder.text) {
		return false, decoder.failure("EOF while parsing a list", decoder.position)
	}
	if decoder.text[decoder.position] == ']' {
		decoder.position++
		return true, nil
	}
	if decoder.text[decoder.position] != ',' {
		return false, decoder.failure("expected `,` or `]`", decoder.position+1)
	}
	decoder.position++
	decoder.whitespace()
	if decoder.position == len(decoder.text) {
		return false, decoder.failure("EOF while parsing a value", decoder.position)
	}
	if decoder.position < len(decoder.text) && decoder.text[decoder.position] == ']' {
		return false, decoder.failure("trailing comma", decoder.position+1)
	}
	return false, nil
}

func (decoder *authorDecoder) authorSequence() (domain.ArticleAuthorDraft, error) {
	var author domain.ArticleAuthorDraft
	decoder.whitespace()
	if decoder.position < len(decoder.text) && decoder.text[decoder.position] == ']' {
		decoder.position++
		return author, decoder.failure("invalid length 0, expected struct ArticleAuthorDraft with 1 element", decoder.position)
	}
	name, err := decoder.token("a string")
	if err != nil {
		return author, err
	}
	author.DisplayName = name.(string)
	decoder.whitespace()
	if decoder.position == len(decoder.text) {
		return author, decoder.failure("EOF while parsing a list", decoder.position)
	}
	if decoder.text[decoder.position] != ']' {
		if decoder.text[decoder.position] == ',' {
			decoder.position++
			decoder.whitespace()
			if decoder.position < len(decoder.text) && decoder.text[decoder.position] == ']' {
				return author, decoder.failure("trailing comma", decoder.position+1)
			}
		}
		return author, decoder.failure("trailing characters", decoder.position+1)
	}
	decoder.position++
	return author, nil
}

func (decoder *authorDecoder) authorNameField(author *domain.ArticleAuthorDraft, hasName bool) error {
	if decoder.text[decoder.position] != '"' {
		return decoder.failure("key must be a string", decoder.position+1)
	}
	key, err := decoder.token("a string")
	if err != nil {
		return err
	}
	if key != "display_name" {
		return decoder.failure("unknown field `"+key.(string)+"`, expected `display_name`", decoder.position)
	}
	if hasName {
		return decoder.failure("duplicate field `display_name`", decoder.position)
	}
	decoder.whitespace()
	if decoder.position == len(decoder.text) {
		return decoder.failure("EOF while parsing an object", decoder.position)
	}
	if decoder.text[decoder.position] != ':' {
		return decoder.failure("expected `:`", decoder.position+1)
	}
	decoder.position++
	name, err := decoder.token("a string")
	if err != nil {
		return err
	}
	author.DisplayName = name.(string)
	return nil
}

func writeAuthorDebugRune(output *strings.Builder, character rune) {
	switch character {
	case 0:
		output.WriteString(`\0`)
	case '\n':
		output.WriteString(`\n`)
	case '\r':
		output.WriteString(`\r`)
	case '\t':
		output.WriteString(`\t`)
	case '\\', '"':
		output.WriteByte('\\')
		output.WriteRune(character)
	default:
		writeAuthorDebugLiteral(output, character)
	}
}

func (decoder *authorDecoder) stringEscape() error {
	escape := decoder.text[decoder.position]
	decoder.position++
	if strings.ContainsRune(`"\/bfnrt`, rune(escape)) {
		return nil
	}
	if escape != 'u' {
		return decoder.failure("invalid escape", decoder.position)
	}
	value, err := decoder.hexEscape()
	if err != nil {
		return err
	}
	if value >= 0xdc00 && value <= 0xdfff {
		return decoder.failure("lone leading surrogate in hex escape", decoder.position)
	}
	if value < 0xd800 || value > 0xdbff {
		return nil
	}
	return decoder.trailingSurrogate()
}

func (decoder *authorDecoder) trailingSurrogate() error {
	for _, expected := range []byte{'\\', 'u'} {
		if decoder.position == len(decoder.text) {
			return decoder.failure("EOF while parsing a string", decoder.position)
		}
		actual := decoder.text[decoder.position]
		decoder.position++
		if actual != expected {
			return decoder.failure("unexpected end of hex escape", decoder.position)
		}
	}
	trailing, err := decoder.hexEscape()
	if err != nil {
		return err
	}
	if trailing < 0xdc00 || trailing > 0xdfff {
		return decoder.failure("lone leading surrogate in hex escape", decoder.position)
	}
	return nil
}

func (decoder *authorDecoder) finishAuthorField() (bool, error) {
	decoder.whitespace()
	if decoder.position == len(decoder.text) {
		return false, decoder.failure("EOF while parsing an object", decoder.position)
	}
	if decoder.text[decoder.position] == '}' {
		decoder.position++
		return true, nil
	}
	if decoder.text[decoder.position] != ',' {
		return false, decoder.failure("expected `,` or `}`", decoder.position+1)
	}
	decoder.position++
	decoder.whitespace()
	if decoder.position < len(decoder.text) && decoder.text[decoder.position] == '}' {
		return false, decoder.failure("trailing comma", decoder.position+1)
	}
	return false, nil
}

func writeAuthorDebugLiteral(output *strings.Builder, character rune) {
	isExtended := unicode.Is(unicode.Mn, character) || unicode.Is(unicode.Me, character)
	if table := unicode.Properties["Other_Grapheme_Extend"]; table != nil && unicode.Is(table, character) {
		isExtended = true
	}
	if !unicode.IsPrint(character) || isExtended {
		output.WriteString(`\u{` + strconv.FormatInt(int64(character), 16) + `}`)
	} else {
		output.WriteRune(character)
	}
}

func (decoder *authorDecoder) closeAuthorObject(hasName bool) (bool, error) {
	if decoder.text[decoder.position] == '}' {
		decoder.position++
		if !hasName {
			return true, decoder.failure("missing field `display_name`", decoder.position)
		}
		return true, nil
	}
	return false, nil
}
