package api

import (
	"bytes"
	"encoding/json"
	"slices"

	storage "github.com/QianFuv/LitRadar/internal/storage/delivery"
)

type manualPushStatus struct {
	JobId                 *string  `json:"job_id"`
	Status                string   `json:"status"`
	Message               string   `json:"message"`
	StartedAt             *float64 `json:"started_at"`
	FinishedAt            *float64 `json:"finished_at"`
	DeadlineAt            *float64 `json:"deadline_at"`
	CancellationRequested bool     `json:"cancellation_requested"`
	CanCancel             bool     `json:"can_cancel"`
	CanRetry              bool     `json:"can_retry"`
	Pushed                int64    `json:"pushed"`
	Selected              int64    `json:"selected"`
	TotalCandidates       *int64   `json:"total_candidates"`
	Summary               string   `json:"summary"`
	FolderId              *int64   `json:"folder_id"`
	FolderName            *string  `json:"folder_name"`
}

func manualOutcome(encoded string) map[string]any {
	kind := structBody("ManualWeeklyPushOutcome", bodyFieldOf("status", bodyType{kind: "raw"}), bodyFieldOf("message", stringBody), bodyFieldOf("pushed", integerBody), bodyFieldOf("selected", integerBody), bodyFieldOf("total_candidates", optionalBody(integerBody)), bodyFieldOf("summary", stringBody), bodyFieldOf("folder_id", optionalBody(integerBody)), bodyFieldOf("folder_name", optionalBody(stringBody)))
	scanner := bodyScanner{body: []byte(encoded)}
	value, err := scanner.typed(kind, "", 0)
	if err != nil || scanner.peek() != 0 || scanner.position != len(scanner.body) {
		return nil
	}
	fields := value.(map[string]any)
	raw := fields["status"].(json.RawMessage)
	var name string
	if json.Unmarshal(raw, &name) != nil || name == "" {
		decoder := json.NewDecoder(bytes.NewReader(raw))
		token, err := decoder.Token()
		if err != nil || token != json.Delim('{') || !decoder.More() {
			return nil
		}
		token, err = decoder.Token()
		if err != nil {
			return nil
		}
		var isString bool
		name, isString = token.(string)
		if !isString {
			return nil
		}
		var unit json.RawMessage
		if decoder.Decode(&unit) != nil || !bytes.Equal(bytes.TrimSpace(unit), []byte("null")) || decoder.More() {
			return nil
		}
	}
	if !slices.Contains([]string{"idle", "pending", "running", "completed", "failed", "cancelled", "timed_out", "unknown"}, name) {
		return nil
	}
	fields["status"] = name
	return fields
}

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
	if run.ResultJson == nil {
		return result
	}
	outcome := manualOutcome(*run.ResultJson)
	if outcome == nil {
		return result
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
	return result
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
