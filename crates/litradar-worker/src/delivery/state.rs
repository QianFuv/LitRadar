//! Delivery progress, completion, and dedupe state helpers.

use super::*;

pub(super) fn should_save_subscriber_progress(config: &RecommendationRunConfig) -> bool {
    config.mode == DeliveryMode::Execute
}
pub(super) fn manual_delivery_state_dir(
    project_root: &Path,
    workflow: DeliveryWorkflow,
) -> PathBuf {
    match workflow {
        DeliveryWorkflow::Notify => project_root.join("data").join("push_state"),
        DeliveryWorkflow::Push => project_root.join("data").join("folder_push_state"),
    }
}
pub(super) fn user_result_from_plan(
    workflow: DeliveryWorkflow,
    plan: &SubscriberDeliveryPlan,
) -> RecommendationUserResult {
    RecommendationUserResult {
        subscriber_id: plan.subscriber_id.clone(),
        selected_count: plan.selected_article_ids.len(),
        pushed_count: plan.selected_article_ids.len(),
        folder_synced_count: if workflow == DeliveryWorkflow::Notify && plan.status == "ok" {
            Some(plan.folder_synced_count)
        } else {
            None
        },
        message_id: plan.message_id.clone(),
        status: plan.status.clone(),
        error: plan.error.clone(),
    }
}

pub(super) fn complete_without_candidates(
    state: &mut RecommendationState,
    run_state: &mut litradar_recommend::RecommendationRunState,
    current_issue_counts: &BTreeMap<String, i64>,
    current_inpress_counts: &BTreeMap<String, i64>,
    pending_issue_keys: &[String],
    pending_inpress_keys: &[String],
    now: &str,
) {
    complete_successfully(
        state,
        run_state,
        current_issue_counts,
        current_inpress_counts,
        pending_issue_keys,
        pending_inpress_keys,
        now,
    );
}

pub(super) fn complete_successfully(
    state: &mut RecommendationState,
    run_state: &mut litradar_recommend::RecommendationRunState,
    current_issue_counts: &BTreeMap<String, i64>,
    current_inpress_counts: &BTreeMap<String, i64>,
    pending_issue_keys: &[String],
    pending_inpress_keys: &[String],
    completed_at: &str,
) {
    run_state.status = "completed".to_string();
    run_state.completed_at = Some(completed_at.to_string());
    run_state.updated_at = completed_at.to_string();
    run_state.done_issue_keys = pending_issue_keys.to_vec();
    run_state.done_inpress_keys = pending_inpress_keys.to_vec();
    run_state.pending_issue_keys = Vec::new();
    run_state.pending_inpress_keys = Vec::new();
    state.status = "completed".to_string();
    state.last_completed_run_at = Some(completed_at.to_string());
    state.snapshot.issue_article_counts = current_issue_counts.clone();
    state.snapshot.inpress_article_counts = current_inpress_counts.clone();
    state.updated_at = completed_at.to_string();
    state.run = Some(run_state.clone());
}
pub(super) fn state_path(state_dir: &Path, db_name: &str) -> PathBuf {
    let stem = db_name
        .strip_suffix(".sqlite")
        .unwrap_or(db_name)
        .to_string();
    state_dir.join(format!("{stem}.json"))
}
