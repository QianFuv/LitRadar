// Package citation serializes favorite snapshots with structural escaping and bounded UTF-8 output.
package citation

import (
	"errors"
	"strconv"
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

// ErrOutputLimit prevents publication of a truncated or oversized citation export.
var ErrOutputLimit = errors.New("citation output limit exceeded")

type boundedText struct {
	text    strings.Builder
	maximum int
	err     error
}

func (output *boundedText) write(value string) {
	if output.err != nil {
		return
	}
	if len(value) > output.maximum-output.text.Len() {
		output.err = ErrOutputLimit
		return
	}
	output.text.WriteString(value)
}

func (output *boundedText) finish() (string, error) {
	if output.err != nil {
		return "", output.err
	}
	return output.text.String(), nil
}

func text(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}

func isLineBreak(character rune) bool {
	return unicode.IsControl(character) || unicode.IsSpace(character) && character != ' '
}

func (output *boundedText) bibtex(value string) {
	for _, character := range value {
		if output.err != nil {
			return
		}
		switch character {
		case '{':
			output.write(`\char123{}`)
		case '}':
			output.write(`\char125{}`)
		case '\\':
			output.write(`\char92{}`)
		case '%', '#', '_', '&', '$':
			output.write(`\` + string(character))
		case '~':
			output.write(`\char126{}`)
		case '^':
			output.write(`\char94{}`)
		default:
			if isLineBreak(character) {
				output.write(" ")
			} else {
				output.write(string(character))
			}
		}
	}
}

func (output *boundedText) bibtexField(name, value string, hasComma bool) {
	output.write("  " + name + " = {")
	output.bibtex(value)
	if hasComma {
		output.write("},\n")
	} else {
		output.write("}\n")
	}
}

// Bibtex preserves record order, deterministic safe keys, field escaping and the inclusive byte limit.
func Bibtex(articles []domain.FavoriteCitation, maximumBytes int) (string, error) {
	output := boundedText{maximum: maximumBytes}
	for index, article := range articles {
		if index > 0 {
			output.write("\n\n")
		}
		output.write("@article{")
		hasKey := false
		for _, character := range text(article.Doi) {
			if character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' {
				output.write(string(character))
				hasKey = true
			}
			if output.err != nil {
				return "", output.err
			}
		}
		if !hasKey {
			output.write("favorite")
		}
		output.write(strconv.Itoa(index+1) + ",\n")
		output.bibtexField("title", text(article.Title), true)
		output.write("  author = {")
		for position, author := range article.Authors {
			if position > 0 {
				output.write(" and ")
			}
			output.bibtex(author)
			if output.err != nil {
				return "", output.err
			}
		}
		output.write("},\n")
		output.bibtexField("journal", text(article.JournalTitle), true)
		output.bibtexField("year", text(article.Date), true)
		output.bibtexField("doi", text(article.Doi), false)
		output.write("}")
		if output.err != nil {
			return "", output.err
		}
	}
	return output.finish()
}

func (output *boundedText) risField(tag, value string) {
	output.write(tag + "  - ")
	for _, character := range value {
		if output.err != nil {
			return
		}
		if isLineBreak(character) {
			output.write(" ")
		} else {
			output.write(string(character))
		}
	}
	output.write("\n")
}

// Ris writes one escaped author tag per author without allowing embedded structural lines.
func Ris(articles []domain.FavoriteCitation, maximumBytes int) (string, error) {
	output := boundedText{maximum: maximumBytes}
	for index, article := range articles {
		if index > 0 {
			output.write("\n\n")
		}
		output.risField("TY", "JOUR")
		output.risField("TI", text(article.Title))
		if len(article.Authors) == 0 {
			output.risField("AU", "")
		}
		for _, author := range article.Authors {
			output.risField("AU", author)
			if output.err != nil {
				return "", output.err
			}
		}
		output.risField("JO", text(article.JournalTitle))
		output.risField("PY", text(article.Date))
		output.risField("DO", text(article.Doi))
		output.write("ER  -")
		if output.err != nil {
			return "", output.err
		}
	}
	return output.finish()
}

func (output *boundedText) xml(value string) {
	for _, character := range value {
		if output.err != nil {
			return
		}
		switch character {
		case '&':
			output.write("&amp;")
		case '<':
			output.write("&lt;")
		case '>':
			output.write("&gt;")
		case '"':
			output.write("&quot;")
		case '\'':
			output.write("&apos;")
		default:
			if character == 9 || character == 10 || character == 13 || character >= 0x20 && character <= 0xd7ff || character >= 0xe000 && character <= 0xfffd || character >= 0x10000 && character <= 0x10ffff {
				output.write(string(character))
			} else {
				output.write(" ")
			}
		}
	}
}

// EndnoteXml preserves the existing field layout and replaces characters forbidden by XML 1.0.
func EndnoteXml(articles []domain.FavoriteCitation, maximumBytes int) (string, error) {
	output := boundedText{maximum: maximumBytes}
	output.write(`<?xml version="1.0" encoding="UTF-8"?><xml><records>`)
	for _, article := range articles {
		output.write("<record><titles><title>")
		output.xml(text(article.Title))
		output.write("</title></titles><contributors><authors>")
		if len(article.Authors) == 0 {
			output.write("<author></author>")
		}
		for _, author := range article.Authors {
			output.write("<author>")
			output.xml(author)
			output.write("</author>")
			if output.err != nil {
				return "", output.err
			}
		}
		output.write("</authors></contributors><dates><year>")
		output.xml(text(article.Date))
		output.write("</year></dates><electronic-resource-num>")
		output.xml(text(article.Doi))
		output.write("</electronic-resource-num></record>")
		if output.err != nil {
			return "", output.err
		}
	}
	output.write("</records></xml>")
	return output.finish()
}
