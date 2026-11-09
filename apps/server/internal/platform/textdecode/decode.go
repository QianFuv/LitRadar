// Package textdecode applies strict HTML charset decoding without silent replacement characters.
package textdecode

import (
	"bytes"
	_ "embed"
	"encoding/binary"
	"errors"
	"regexp"
	"sort"
	"strings"
	"unicode/utf8"

	"golang.org/x/net/html/charset"
	"golang.org/x/text/encoding"
	"golang.org/x/text/encoding/simplifiedchinese"
)

var charsetPattern = regexp.MustCompile(`(?i)charset\s*=\s*["']?\s*([a-z0-9_-]+)`)
var metaPattern = regexp.MustCompile(`(?is)<meta\b[^>]*>`)

// ErrEncoding reports an unknown charset or malformed byte sequence.
var ErrEncoding = errors.New("invalid text encoding")

//go:embed gb18030.bin
var gb18030Corrections []byte

func correctedGb18030(unit []byte) (rune, bool) {
	var key uint32
	for _, value := range unit {
		key = key<<8 | uint32(value)
	}
	index := sort.Search(len(gb18030Corrections)/8, func(index int) bool {
		return binary.BigEndian.Uint32(gb18030Corrections[index*8:]) >= key
	})
	if index*8 == len(gb18030Corrections) || binary.BigEndian.Uint32(gb18030Corrections[index*8:]) != key {
		return 0, false
	}
	return int32(binary.BigEndian.Uint32(gb18030Corrections[index*8+4:])), true
}

// Html selects BOM, then HTTP charset, then the first charset in a meta tag within 4096 bytes.
func Html(body []byte, contentType string) (string, error) {
	label := "utf-8"
	if bytes.HasPrefix(body, []byte{0xef, 0xbb, 0xbf}) {
		return Decode(body[3:], "utf-8")
	}
	if bytes.HasPrefix(body, []byte{0xff, 0xfe}) {
		return Decode(body[2:], "utf-16le")
	}
	if bytes.HasPrefix(body, []byte{0xfe, 0xff}) {
		return Decode(body[2:], "utf-16be")
	}
	if match := charsetPattern.FindStringSubmatch(contentType); match != nil {
		label = match[1]
	} else {
		for _, tag := range metaPattern.FindAll(body[:min(len(body), 4096)], -1) {
			if match := charsetPattern.FindSubmatch(tag); match != nil {
				label = string(match[1])
				break
			}
		}
	}
	return Decode(body, label)
}

// Decode rejects malformed sequences while retaining a legitimately encoded Unicode replacement rune.
func Decode(body []byte, label string) (string, error) {
	encoding, name := charset.Lookup(label)
	if encoding == nil {
		return "", ErrEncoding
	}
	switch name {
	case "utf-8":
		if !utf8.Valid(body) {
			return "", ErrEncoding
		}
		return string(body), nil
	case "utf-16le", "utf-16be":
		if !validUtf16(body, name == "utf-16le") {
			return "", ErrEncoding
		}
	case "gbk", "gb18030":
		return decodeGb18030(body)
	case "windows-1252":
		return decodeWindows1252(body, encoding)
	}
	return decodeStrictBytes(body, name, encoding)
}

// validUtf16 rejects odd lengths, isolated low surrogates and incomplete high-surrogate pairs.
func validUtf16(body []byte, littleEndian bool) bool {
	if len(body)%2 != 0 {
		return false
	}
	var order binary.ByteOrder = binary.BigEndian
	if littleEndian {
		order = binary.LittleEndian
	}
	for offset := 0; offset < len(body); offset += 2 {
		value := order.Uint16(body[offset:])
		if value >= 0xdc00 && value <= 0xdfff {
			return false
		}
		if value >= 0xd800 && value <= 0xdbff {
			offset += 2
			if !validUtf16LowSurrogate(body, offset, order) {
				return false
			}
		}
	}
	return true
}

// GBK's WHATWG decoder shares GB18030 decoding; x/text's separate GBK decoder does not.
func decodeGb18030(body []byte) (string, error) {
	var result strings.Builder
	decoder := simplifiedchinese.GB18030.NewDecoder()
	for offset := 0; offset < len(body); {
		length, ok := gb18030UnitLength(body, offset)
		if !ok {
			return "", ErrEncoding
		}
		unit := body[offset : offset+length]
		if value, found := correctedGb18030(unit); found {
			if value < 0 {
				return "", ErrEncoding
			}
			result.WriteRune(value)
			offset += length
			continue
		}
		decoded, err := decoder.Bytes(unit)
		if err != nil {
			return "", ErrEncoding
		}
		if bytes.Contains(decoded, []byte("�")) {
			return "", ErrEncoding
		}
		result.Write(decoded)
		offset += length
	}
	return result.String(), nil
}

// decodeWindows1252 preserves undefined single-byte values as their control runes.
func decodeWindows1252(body []byte, codec encoding.Encoding) (string, error) {
	var result strings.Builder
	for _, value := range body {
		switch value {
		case 0x81, 0x8d, 0x8f, 0x90, 0x9d:
			result.WriteRune(rune(value))
		default:
			decoded, err := codec.NewDecoder().Bytes([]byte{value})
			if err != nil {
				return "", ErrEncoding
			}
			result.Write(decoded)
		}
	}
	return result.String(), nil
}

// decodeStrictBytes rejects decoder replacement outside already validated UTF-16.
func decodeStrictBytes(body []byte, name string, codec encoding.Encoding) (string, error) {
	decoded, err := codec.NewDecoder().Bytes(body)
	if err != nil {
		return "", ErrEncoding
	}
	if name != "utf-16le" && name != "utf-16be" && strings.ContainsRune(string(decoded), utf8.RuneError) {
		return "", ErrEncoding
	}
	return string(decoded), nil
}

// validUtf16LowSurrogate checks the second code unit after the caller advances past a high surrogate.
func validUtf16LowSurrogate(body []byte, offset int, order binary.ByteOrder) bool {
	if offset >= len(body) {
		return false
	}
	low := order.Uint16(body[offset:])
	if low < 0xdc00 || low > 0xdfff {
		return false
	}
	return true
}

// gb18030UnitLength admits single-byte units or validates the complete multibyte shape.
func gb18030UnitLength(body []byte, offset int) (int, bool) {
	first := body[offset]
	if first >= 0x81 && first <= 0xfe {
		return gb18030MultibyteLength(body, offset)
	}
	return 1, first != 0xff
}

// gb18030MultibyteLength distinguishes decimal four-byte units from legal two-byte trails.
func gb18030MultibyteLength(body []byte, offset int) (int, bool) {
	if offset+1 >= len(body) {
		return 0, false
	}
	second := body[offset+1]
	if second >= 0x30 && second <= 0x39 {
		return 4, validGb18030FourByte(body, offset)
	}
	if second >= 0x40 && second <= 0xfe && second != 0x7f {
		return 2, true
	}
	return 0, false
}

// validGb18030FourByte requires the final lead and decimal trail after checking the complete length.
func validGb18030FourByte(body []byte, offset int) bool {
	return offset+3 < len(body) && body[offset+2] >= 0x81 && body[offset+2] <= 0xfe && body[offset+3] >= 0x30 && body[offset+3] <= 0x39
}
