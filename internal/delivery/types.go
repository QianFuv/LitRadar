package delivery

import (
	"errors"
	"log/slog"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

var ErrBusy = errors.New("Delivery workflow is already running")

// RunConfig binds one database delivery to its explicit invocation and optional parent execution boundary.
type RunConfig struct {
	AuthDbPath          string
	SecretCodec         *secrets.Codec
	IndexDbPath         string
	DbName              string
	ChangesFile         *string
	AttemptId           *string
	AiModel             *string
	MaxCandidates       *int
	TimeoutSeconds      uint64
	RetryAttempts       int
	DedupeRetentionDays int64
	Mode                store.RunMode
	Workflow            store.Workflow
	Trigger             store.TriggerKind
	ExecutionControl    *domain.ExecutionControl
}

func (config RunConfig) String() string       { return "RecommendationRunConfig([REDACTED])" }
func (config RunConfig) GoString() string     { return config.String() }
func (config RunConfig) LogValue() slog.Value { return slog.StringValue(config.String()) }

// FavoriteWritePlan preserves the owner, folder and source database of an accepted article.
type FavoriteWritePlan struct {
	UserId    int64  `json:"user_id"`
	FolderId  int64  `json:"folder_id"`
	ArticleId int64  `json:"article_id"`
	DbName    string `json:"db_name"`
}

// SubscriberPlan records planned content separately from observed external delivery results.
type SubscriberPlan struct {
	SubscriberId       string              `json:"subscriber_id"`
	DeliveryMethod     string              `json:"delivery_method"`
	Status             string              `json:"status"`
	Error              *string             `json:"error"`
	SelectedArticleIds []int64             `json:"selected_article_ids"`
	MessageTitle       *string             `json:"message_title"`
	MessageContent     *string             `json:"message_content"`
	MessageId          *string             `json:"message_id"`
	FavoriteWrites     []FavoriteWritePlan `json:"favorite_writes"`
	FolderSyncedCount  uint64              `json:"folder_synced_count"`
	WouldSendPushplus  bool                `json:"would_send_pushplus"`
}

func (plan SubscriberPlan) String() string       { return "SubscriberDeliveryPlan([REDACTED])" }
func (plan SubscriberPlan) GoString() string     { return plan.String() }
func (plan SubscriberPlan) LogValue() slog.Value { return slog.StringValue(plan.String()) }

// RunOutcome is the public result of one durable database delivery invocation.
type RunOutcome struct {
	DbName              string           `json:"db_name"`
	Workflow            store.Workflow   `json:"workflow"`
	Mode                store.RunMode    `json:"mode"`
	Status              string           `json:"status"`
	DeliveryRunId       int64            `json:"delivery_run_id"`
	CandidateArticleIds []int64          `json:"candidate_article_ids"`
	Subscribers         []SubscriberPlan `json:"subscribers"`
}

func (outcome RunOutcome) String() string       { return "RecommendationRunOutcome([REDACTED])" }
func (outcome RunOutcome) GoString() string     { return outcome.String() }
func (outcome RunOutcome) LogValue() slog.Value { return slog.StringValue(outcome.String()) }
