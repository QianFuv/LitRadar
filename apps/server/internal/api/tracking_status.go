package api

import (
	"bytes"
	"encoding/json"
	"slices"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
)

type manualPushStatus struct {
	JobId                 *string  `json:"job_id" required:"false" nullable:"true"`
	Status                string   `json:"status" nullable:"false" enum:"idle,pending,running,completed,failed,cancelled,timed_out,unknown"`
	Message               string   `json:"message" nullable:"false"`
	StartedAt             *float64 `json:"started_at" required:"false" nullable:"true" format:"double"`
	FinishedAt            *float64 `json:"finished_at" required:"false" nullable:"true" format:"double"`
	DeadlineAt            *float64 `json:"deadline_at" required:"false" nullable:"true" format:"double"`
	CancellationRequested bool     `json:"cancellation_requested" nullable:"false"`
	CanCancel             bool     `json:"can_cancel" nullable:"false"`
	CanRetry              bool     `json:"can_retry" nullable:"false"`
	Pushed                int64    `json:"pushed" nullable:"false" format:"int64"`
	Selected              int64    `json:"selected" nullable:"false" format:"int64"`
	TotalCandidates       *int64   `json:"total_candidates" required:"false" nullable:"true" format:"int64"`
	Summary               string   `json:"summary" nullable:"false"`
	FolderId              *int64   `json:"folder_id" required:"false" nullable:"true" format:"int64"`
	FolderName            *string  `json:"folder_name" required:"false" nullable:"true"`
}

// manualOutcome admits a complete typed result and a strictly represented status.
func manualOutcome(encoded string) map[string]any {
	fields, err := storedFields(encoded, []string{"status", "message", "pushed", "selected", "total_candidates", "summary", "folder_id", "folder_name"})
	if err != nil {
		return nil
	}
	required := map[string]any{"message": new(string), "pushed": new(int64), "selected": new(int64), "summary": new(string)}
	result := map[string]any{}
	for name, target := range required {
		raw, exists := fields[name]
		if !exists || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return nil
		}
		if err := json.Unmarshal(raw, target); err != nil {
			return nil
		}
		switch value := target.(type) {
		case *string:
			result[name] = *value
		case *int64:
			result[name] = *value
		}
	}
	for _, name := range []string{"total_candidates", "folder_id", "folder_name"} {
		raw, exists := fields[name]
		if !exists {
			continue
		}
		if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			continue
		}
		if name == "folder_name" {
			var value string
			if err := json.Unmarshal(raw, &value); err != nil {
				return nil
			}
			result[name] = value
		} else {
			var value int64
			if err := json.Unmarshal(raw, &value); err != nil {
				return nil
			}
			result[name] = value
		}
	}
	name, ok := manualOutcomeStatus(fields["status"])
	if !ok {
		return nil
	}
	result["status"] = name
	return result
}

func manualOutcomeStatus(raw json.RawMessage) (string, bool) {
	var name string
	if json.Unmarshal(raw, &name) != nil || name == "" {
		var isValid bool
		name, isValid = manualOutcomeUnit(raw)
		if !isValid {
			return "", false
		}
	}
	return name, slices.Contains([]string{"idle", "pending", "running", "completed", "failed", "cancelled", "timed_out", "unknown"}, name)
}

// manualOutcomeUnit rejects extra members and non-null unit payloads.
func manualOutcomeUnit(raw json.RawMessage) (string, bool) {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') || !decoder.More() {
		return "", false
	}
	token, err = decoder.Token()
	if err != nil {
		return "", false
	}
	name, isString := token.(string)
	if !isString {
		return "", false
	}
	var unit json.RawMessage
	if decoder.Decode(&unit) != nil || !bytes.Equal(bytes.TrimSpace(unit), []byte("null")) || decoder.More() {
		return "", false
	}
	return name, true
}

// manualStatus projects durable run state before optional decoded result metrics.
func manualStatus(run *storage.RunRecord) manualPushStatus {
	result := manualPushStatus{Status: "idle", Message: "No manual push task is available"}
	if run == nil {
		return result
	}
	result.JobId = &run.ExternalId
	result.Status = string(run.Status)
	switch run.Status {
	case storage.RunStatusQueued:
		result.Status = "pending"
	case storage.RunStatusClaimed, storage.RunStatusRunning, storage.RunStatusCancelling:
		result.Status = "running"
	case storage.RunStatusCompleted, storage.RunStatusSkipped:
		result.Status = "completed"
	}
	result.Message = map[storage.RunStatus]string{storage.RunStatusQueued: "Manual push is queued", storage.RunStatusClaimed: "Manual push is running", storage.RunStatusRunning: "Manual push is running", storage.RunStatusCancelling: "Manual push cancellation is pending", storage.RunStatusCompleted: "Manual push completed", storage.RunStatusSkipped: "Manual push completed without applicable work", storage.RunStatusFailed: "Manual push failed", storage.RunStatusCancelled: "Manual push was cancelled", storage.RunStatusTimedOut: "Manual push exceeded its deadline", storage.RunStatusUnknown: "Manual push outcome is unknown; review delivery state before retrying"}[run.Status]
	if run.ErrorCode != nil {
		if message := manualErrorMessage[*run.ErrorCode]; message != "" {
			result.Message = message
		}
	}
	result.StartedAt, result.FinishedAt, result.DeadlineAt = run.StartedAt, run.FinishedAt, run.DeadlineAt
	result.CancellationRequested = run.CancellationRequested
	result.CanCancel = !run.Status.IsTerminal() && !run.CancellationRequested
	result.CanRetry = run.Status == storage.RunStatusFailed || run.Status == storage.RunStatusCancelled || run.Status == storage.RunStatusTimedOut
	applyManualOutcome(&result, run)
	return result
}

// applyManualOutcome projects valid metrics while limiting message overrides to completed runs.
func applyManualOutcome(result *manualPushStatus, run *storage.RunRecord) {
	if run.ResultJson == nil {
		return
	}
	outcome := manualOutcome(*run.ResultJson)
	if outcome == nil {
		return
	}
	if (run.Status == storage.RunStatusCompleted || run.Status == storage.RunStatusSkipped) && outcome["status"] == "completed" && outcome["message"].(string) != "" {
		result.Message = outcome["message"].(string)
	}
	result.Pushed, result.Selected, result.Summary = outcome["pushed"].(int64), outcome["selected"].(int64), outcome["summary"].(string)
	if value, ok := outcome["total_candidates"].(int64); ok {
		result.TotalCandidates = &value
	}
	if value, ok := outcome["folder_id"].(int64); ok {
		result.FolderId = &value
	}
	if value, ok := outcome["folder_name"].(string); ok {
		result.FolderName = &value
	}
}

var manualErrorMessage = map[string]string{
	"invalid_job_context":            "手动推送任务配置无效，请重新创建任务",
	"cancelled":                      "手动推送已取消，可重新发起",
	"deadline_exceeded":              "手动推送已超时，请稍后重试",
	"ambiguous_delivery":             "推送结果不确定，请先检查接收端，再确认未知结果",
	"delivery_failed":                "手动推送未完成，请检查 AI、PushPlus 或跟踪文件夹设置后重试",
	"cancellation_state_unavailable": "无法确认取消状态，请稍后查看任务状态后再重试",
	"ai_request_budget_exhausted":    "AI 请求次数已用尽，请检查 AI 配置或稍后重试",
	"workflow_busy":                  "已有投递任务占用资源，请稍后重试",
	"index_storage_failed":           "文章索引暂时无法读取，请稍后重试",
	"business_storage_failed":        "通知设置暂时无法读取，请稍后重试",
	"delivery_storage_failed":        "投递状态暂时无法保存，请稍后重试",
	"auth_storage_failed":            "账户配置暂时无法读取，请稍后重试",
	"recommendation_failed":          "文章筛选失败，请检查推荐设置后重试",
	"ai_failed":                      "AI 摘要生成失败，请检查 AI 配置后重试",
	"pushplus_failed":                "PushPlus 推送失败，请检查令牌和网络后重试",
	"manual_validation_failed":       "手动推送配置无效，请检查通知设置后重试",
	"spawn_or_assign_failed":         "推送任务暂时无法启动，请稍后重试",
	"forced_cancellation_unknown":    "强制取消后推送结果不确定，请先检查接收端，再确认未知结果",
	"forced_deadline_unknown":        "强制结束超时任务后结果不确定，请先检查接收端，再确认未知结果",
	"forced_shutdown_unknown":        "服务停止时推送结果不确定，请先检查接收端，再确认未知结果",
	"forced_termination_unknown":     "任务异常中断后推送结果不确定，请先检查接收端，再确认未知结果",
}
