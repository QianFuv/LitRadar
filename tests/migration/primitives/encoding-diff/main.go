// Command encoding-diff measures the pinned Go decoder against an independent Rust oracle.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"fmt"
	"os"
	"unicode/utf8"

	"golang.org/x/text/encoding/simplifiedchinese"
)

func main() {
	if len(os.Args) != 3 {
		panic("expected oracle and output paths")
	}
	oracle, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	if fmt.Sprintf("%x", sha256.Sum256(oracle)) != "d930c412b9d64eae3b126448db4f0aa65d18b8e882f639423bb2219ff7a7e66f" {
		panic("unexpected frozen oracle")
	}
	position := 8
	var corrections bytes.Buffer
	decoder := simplifiedchinese.GB18030.NewDecoder()
	twoByteCount, fourByteCount := 0, 0
	compare := func(unit []byte) {
		expected := int32(binary.LittleEndian.Uint32(oracle[position:]))
		position += 4
		decoded, err := decoder.Bytes(unit)
		actual, size := utf8.DecodeRune(decoded)
		if err != nil || size != len(decoded) || actual == utf8.RuneError {
			actual = -1
		}
		if expected == actual {
			return
		}
		var key uint32
		for _, value := range unit {
			key = key<<8 | uint32(value)
		}
		binary.Write(&corrections, binary.BigEndian, key)
		binary.Write(&corrections, binary.BigEndian, expected)
		if len(unit) == 2 {
			twoByteCount++
		} else {
			fourByteCount++
		}
	}
	for first := 0x81; first <= 0xfe; first++ {
		for second := 0x40; second <= 0xfe; second++ {
			if second != 0x7f {
				compare([]byte{byte(first), byte(second)})
			}
		}
	}
	for first := 0x81; first <= 0xfe; first++ {
		for second := 0x30; second <= 0x39; second++ {
			for third := 0x81; third <= 0xfe; third++ {
				for fourth := 0x30; fourth <= 0x39; fourth++ {
					compare([]byte{byte(first), byte(second), byte(third), byte(fourth)})
				}
			}
		}
	}
	if position != len(oracle) {
		panic("unread oracle bytes")
	}
	if err := os.WriteFile(os.Args[2], corrections.Bytes(), 0o644); err != nil {
		panic(err)
	}
	fmt.Printf("two-byte=%d four-byte=%d correction-bytes=%d sha256=%x\n", twoByteCount, fourByteCount, corrections.Len(), sha256.Sum256(corrections.Bytes()))
}
