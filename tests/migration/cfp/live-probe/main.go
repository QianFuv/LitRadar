// Command live-probe performs one bounded public discovery read per automatic CFP adapter.
package main

import (
	"context"
	"encoding/json"
	"os"
	"time"

	"github.com/QianFuv/LitRadar/internal/cfp"
)

func main() {
	selected := map[cfp.Adapter]bool{}
	observations := []map[string]any{}
	for _, config := range cfp.Registry() {
		if !config.CanRefresh() || selected[config.Adapter] {
			continue
		}
		selected[config.Adapter] = true
		transport := cfp.NewHttpTransport()
		started := time.Now().UTC()
		document, err := transport.Fetch(context.Background(), config, config.DiscoveryUrl, time.Now().Add(20*time.Second))
		transport.Close()
		observation := map[string]any{"adapter": config.Adapter, "sourceKey": config.SourceKey, "requestedUrl": config.DiscoveryUrl, "started": started, "finished": time.Now().UTC(), "status": "Blocked"}
		if err != nil {
			observation["error"] = err.Error()
		} else {
			observation["finalUrl"] = document.FinalUrl
			observation["bytes"] = len(document.Text)
			observation["status"] = "Fetched"
			parsed, parseError := cfp.ParsePage(config, document, started.Format("2006-01-02"), true)
			if parseError != nil {
				observation["parseError"] = parseError.Error()
			} else {
				observation["status"] = "Parsed"
				observation["notices"] = len(parsed.Sources)
				observation["detailPages"] = len(parsed.DetailUrls)
			}
		}
		observations = append(observations, observation)
	}
	json.NewEncoder(os.Stdout).Encode(observations)
}
