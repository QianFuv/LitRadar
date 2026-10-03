// Command secret-candidate emits synthetic Go envelopes for independent Rust verification.
package main

import (
	"bytes"
	"encoding/json"
	"os"
	"strings"

	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

type observation struct {
	Key       byte   `json:"key"`
	Stored    string `json:"stored"`
	Context   string `json:"context"`
	Plaintext string `json:"plaintext"`
	Reject    bool   `json:"reject"`
}

func main() {
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		panic(err)
	}
	defer codec.Close()
	encoder := json.NewEncoder(os.Stdout)
	for _, associated := range []string{secrets.NotificationContext(2, "ai_api_key"), secrets.RuntimeContext("provider_proxy_url"), secrets.CnkiContext(2), secrets.PoolReferenceContext("openalex_api_key_pool")} {
		plaintext := "synthetic-密钥-é-🔑"
		stored, err := codec.Encrypt(plaintext, associated)
		if err != nil {
			panic(err)
		}
		cases := []observation{{42, stored, associated, plaintext, false}, {43, stored, associated, plaintext, true}, {42, stored, associated + ":wrong", plaintext, true}}
		for _, field := range []int{2, 3} {
			for _, newline := range []string{"\r", "\n"} {
				parts := strings.Split(stored, ":")
				parts[field] = parts[field][:1] + newline + parts[field][1:]
				cases = append(cases, observation{42, strings.Join(parts, ":"), associated, plaintext, true})
			}
		}
		for _, value := range cases {
			if err := encoder.Encode(value); err != nil {
				panic(err)
			}
		}
	}
}
