package runtime

import (
	_ "embed"
	"path"
	"strings"
)

//go:embed mime/types.tsv
var mimeData string

var frontendMimeTypes = loadMimeTypes()

func loadMimeTypes() map[string]string {
	result := map[string]string{}
	for _, row := range strings.Split(mimeData, "\n") {
		if extension, mediaType, exists := strings.Cut(row, "\t"); exists {
			result[extension] = mediaType
		}
	}
	return result
}

func frontendMediaType(name string) string {
	extension := strings.TrimPrefix(strings.ToLower(path.Ext(name)), ".")
	if mediaType, exists := frontendMimeTypes[extension]; exists {
		return mediaType
	}
	return "application/octet-stream"
}
