module github.com/QianFuv/LitRadar

go 1.26.0

toolchain go1.27.3

require (
	github.com/PuerkitoBio/goquery v1.13.0
	github.com/danieljoos/wincred v1.2.3
	github.com/mattn/go-sqlite3 v1.14.52
	github.com/modelcontextprotocol/go-sdk v1.8.0
	github.com/nlnwa/whatwg-url v0.6.2
	github.com/wailsapp/wails/v3 v3.1.0
	golang.org/x/crypto v0.57.0
	golang.org/x/net v0.60.0
	golang.org/x/oauth2 v0.35.0
	golang.org/x/sys v0.48.0
	golang.org/x/text v0.42.0
)

require (
	github.com/andybalholm/cascadia v1.3.4 // indirect
	github.com/bits-and-blooms/bitset v1.20.0 // indirect
	github.com/coder/websocket v1.8.14 // indirect
	github.com/ebitengine/purego v0.11.1 // indirect
	github.com/go-ole/go-ole v1.3.0 // indirect
	github.com/godbus/dbus/v5 v5.2.2 // indirect
	github.com/google/jsonschema-go v0.4.3 // indirect
	github.com/segmentio/asm v1.1.3 // indirect
	github.com/segmentio/encoding v0.5.4 // indirect
	github.com/yosida95/uritemplate/v3 v3.0.2 // indirect
	golang.org/x/sync v0.23.0 // indirect
	golang.org/x/term v0.46.0 // indirect
	golang.org/x/time v0.15.0 // indirect
)

replace github.com/mattn/go-sqlite3 => ./third_party/go-sqlite3

replace github.com/modelcontextprotocol/go-sdk => ./third_party/go-sdk
