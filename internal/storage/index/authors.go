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
	var token json.Token
	var err error
	if decoder.text[start] == '"' {
		token, err = decoder.stringToken()
		if err != nil {
			return nil, err
		}
	} else {
		stream := json.NewDecoder(strings.NewReader(decoder.text[start:]))
		stream.UseNumber()
		token, err = stream.Token()
		if err == nil {
			decoder.position += int(stream.InputOffset())
		}
	}
	if err != nil {
		if err == io.EOF || err == io.ErrUnexpectedEOF {
			return nil, decoder.failure("EOF while parsing a value", len(decoder.text))
		}
		var syntax *json.SyntaxError
		if value, ok := err.(*json.SyntaxError); ok {
			syntax = value
		}
		position := start + 1
		if syntax != nil {
			position = start + int(syntax.Offset)
		}
		return nil, decoder.failure("expected value", position)
	}
	var description string
	switch value := token.(type) {
	case nil:
		description = "null"
	case bool:
		description = fmt.Sprintf("boolean `%t`", value)
	case json.Number:
		number, err := domain.ParseNumber(value)
		if err != nil {
			return nil, decoder.failure("number out of range", decoder.position)
		}
		kind := "integer"
		_, isUnsigned := number.AsUint64()
		_, isSigned := number.AsInt64()
		if !isUnsigned && !isSigned {
			kind = "floating point"
		}
		description = kind + " `" + number.String() + "`"
	case string:
		description = "string " + rustDebugString(value)
	case json.Delim:
		if value == '[' {
			description = "sequence"
		} else {
			description = "map"
		}
	}
	isAccepted := expected == "a sequence" && token == json.Delim('[') || expected == "struct ArticleAuthorDraft" && (token == json.Delim('[') || token == json.Delim('{'))
	if _, isString := token.(string); expected == "a string" && isString {
		isAccepted = true
	}
	if !isAccepted {
		position := decoder.position
		if _, isDelimiter := token.(json.Delim); isDelimiter {
			position = start
		}
		return nil, decoder.failure("invalid type: "+description+", expected "+expected, position)
	}
	return token, nil
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
		decoder.whitespace()
		if decoder.position == len(text) {
			return nil, decoder.failure("EOF while parsing a list", decoder.position)
		}
		if text[decoder.position] == ']' {
			decoder.position++
			break
		}
		if text[decoder.position] != ',' {
			return nil, decoder.failure("expected `,` or `]`", decoder.position+1)
		}
		decoder.position++
		decoder.whitespace()
		if decoder.position == len(text) {
			return nil, decoder.failure("EOF while parsing a value", decoder.position)
		}
		if decoder.position < len(text) && text[decoder.position] == ']' {
			return nil, decoder.failure("trailing comma", decoder.position+1)
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
	hasName := false
	for {
		decoder.whitespace()
		if decoder.position == len(decoder.text) {
			return author, decoder.failure("EOF while parsing an object", decoder.position)
		}
		if decoder.text[decoder.position] == '}' {
			decoder.position++
			if !hasName {
				return author, decoder.failure("missing field `display_name`", decoder.position)
			}
			return author, nil
		}
		if decoder.text[decoder.position] != '"' {
			return author, decoder.failure("key must be a string", decoder.position+1)
		}
		key, err := decoder.token("a string")
		if err != nil {
			return author, err
		}
		if key != "display_name" {
			return author, decoder.failure("unknown field `"+key.(string)+"`, expected `display_name`", decoder.position)
		}
		if hasName {
			return author, decoder.failure("duplicate field `display_name`", decoder.position)
		}
		decoder.whitespace()
		if decoder.position == len(decoder.text) {
			return author, decoder.failure("EOF while parsing an object", decoder.position)
		}
		if decoder.text[decoder.position] != ':' {
			return author, decoder.failure("expected `:`", decoder.position+1)
		}
		decoder.position++
		name, err := decoder.token("a string")
		if err != nil {
			return author, err
		}
		author.DisplayName, hasName = name.(string), true
		decoder.whitespace()
		if decoder.position == len(decoder.text) {
			return author, decoder.failure("EOF while parsing an object", decoder.position)
		}
		if decoder.text[decoder.position] == '}' {
			decoder.position++
			return author, nil
		}
		if decoder.text[decoder.position] != ',' {
			return author, decoder.failure("expected `,` or `}`", decoder.position+1)
		}
		decoder.position++
		decoder.whitespace()
		if decoder.position < len(decoder.text) && decoder.text[decoder.position] == '}' {
			return author, decoder.failure("trailing comma", decoder.position+1)
		}
	}
}

func rustDebugString(value string) string {
	var output strings.Builder
	output.WriteByte('"')
	for _, character := range value {
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
		escape := decoder.text[decoder.position]
		decoder.position++
		if strings.ContainsRune(`"\/bfnrt`, rune(escape)) {
			continue
		}
		if escape != 'u' {
			return "", decoder.failure("invalid escape", decoder.position)
		}
		value, err := decoder.hexEscape()
		if err != nil {
			return "", err
		}
		if value >= 0xdc00 && value <= 0xdfff {
			return "", decoder.failure("lone leading surrogate in hex escape", decoder.position)
		}
		if value < 0xd800 || value > 0xdbff {
			continue
		}
		for _, expected := range []byte{'\\', 'u'} {
			if decoder.position == len(decoder.text) {
				return "", decoder.failure("EOF while parsing a string", decoder.position)
			}
			actual := decoder.text[decoder.position]
			decoder.position++
			if actual != expected {
				return "", decoder.failure("unexpected end of hex escape", decoder.position)
			}
		}
		trailing, err := decoder.hexEscape()
		if err != nil {
			return "", err
		}
		if trailing < 0xdc00 || trailing > 0xdfff {
			return "", decoder.failure("lone leading surrogate in hex escape", decoder.position)
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
