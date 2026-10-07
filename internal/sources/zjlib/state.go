package zjlib

import (
	"encoding/base64"
	"fmt"
	"strings"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func jwtExpiration(token string) *int64 {
	parts := strings.SplitN(token, ".", 3)
	if len(parts) < 2 {
		return nil
	}
	raw, ok := decodeBase64Url(parts[1])
	if !ok {
		return nil
	}
	body, err := transport.ParseJson(raw)
	if err != nil {
		return nil
	}
	return int64Field(field(body, "exp"))
}

// decodeBase64Url preserves embedded padding and incomplete trailing groups.
func decodeBase64Url(value string) ([]byte, bool) {
	var bits uint32
	var count uint8
	result := []byte{}
	for _, character := range []byte(value) {
		if character == '=' {
			continue
		}
		digit, ok := base64UrlDigit(character)
		if !ok {
			return nil, false
		}
		bits = bits<<6 | uint32(digit)
		count += 6
		for count >= 8 {
			count -= 8
			result = append(result, byte(bits>>count))
		}
	}
	return result, true
}
func unsignedJwt(expires int64) string {
	return base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"none"}`)) + "." + base64.RawURLEncoding.EncodeToString([]byte(fmt.Sprintf(`{"exp":%d}`, expires))) + "."
}

// base64UrlDigit maps only the original URL alphabet, leaving padding to the owner.
func base64UrlDigit(character byte) (byte, bool) {
	var digit byte
	switch {
	case character >= 'A' && character <= 'Z':
		digit = character - 'A'
	case character >= 'a' && character <= 'z':
		digit = character - 'a' + 26
	case character >= '0' && character <= '9':
		digit = character - '0' + 52
	case character == '-':
		digit = 62
	case character == '_':
		digit = 63
	default:
		return 0, false
	}
	return digit, true
}
