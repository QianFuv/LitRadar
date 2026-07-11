//! AI-backed delivery candidate selection.

use super::*;

use crate::retry::{bounded_retry_attempts, bounded_retry_attempts_from_i64};

const MAX_AI_SELECTION_ROUNDS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AiSelectionOutcome {
    pub(super) accepted: Vec<RankedSelectionInfo>,
    pub(super) summary: String,
    pub(super) skip_reason: Option<String>,
}

pub(super) trait DeliveryAiSelector {
    fn select_for_subscriber(
        &mut self,
        request: DeliveryAiSelectionRequest<'_>,
    ) -> Result<AiSelectionOutcome, DeliveryError>;
}

pub(super) struct DeliveryAiSelectionRequest<'a> {
    pub(super) subscriber: &'a NotificationSubscriberInfo,
    pub(super) global_config: &'a NotificationGlobalConfig,
    pub(super) defaults: &'a NotificationDefaults,
    pub(super) override_model: Option<&'a str>,
    pub(super) candidates_for_model: &'a [ArticleCandidateInfo],
    pub(super) candidates_by_id: &'a BTreeMap<i64, ArticleCandidateInfo>,
    pub(super) delivery_dedupe: &'a BTreeMap<String, String>,
}

pub(super) trait AiSelectionClient {
    fn select_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        defaults: &NotificationDefaults,
        candidates: &[ArticleCandidateInfo],
    ) -> Result<SelectionResultInfo, String>;

    fn summarize_selected_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        selected_candidates: &[ArticleCandidateInfo],
    ) -> Result<String, String>;
}

pub(super) trait AiSelectionClientFactory {
    fn build_client(
        &mut self,
        config: &AiRuntimeConfig,
        retry_attempts: usize,
        temperature: f64,
    ) -> Result<Box<dyn AiSelectionClient>, String>;
}

pub(super) struct DefaultDeliveryAiSelector<F: AiSelectionClientFactory> {
    factory: F,
    retry_attempts: usize,
    max_rounds: usize,
}

impl DefaultDeliveryAiSelector<LiveAiSelectionClientFactory> {
    pub(super) fn live(timeout_seconds: u64, retry_attempts: usize) -> Self {
        Self {
            factory: LiveAiSelectionClientFactory { timeout_seconds },
            retry_attempts,
            max_rounds: MAX_AI_SELECTION_ROUNDS,
        }
    }
}

impl<F: AiSelectionClientFactory> DefaultDeliveryAiSelector<F> {
    #[cfg(test)]
    fn new(factory: F, retry_attempts: usize, max_rounds: usize) -> Self {
        Self {
            factory,
            retry_attempts,
            max_rounds,
        }
    }
}

impl<F: AiSelectionClientFactory> DeliveryAiSelector for DefaultDeliveryAiSelector<F> {
    fn select_for_subscriber(
        &mut self,
        request: DeliveryAiSelectionRequest<'_>,
    ) -> Result<AiSelectionOutcome, DeliveryError> {
        let DeliveryAiSelectionRequest {
            subscriber,
            global_config,
            defaults,
            override_model,
            candidates_for_model,
            candidates_by_id,
            delivery_dedupe,
        } = request;
        if !has_selection_preferences(subscriber) {
            return Ok(skipped_ai_selection("No keywords or directions configured"));
        }
        let ai_configs =
            resolve_ai_runtime_configs(subscriber, global_config, defaults, override_model);
        if ai_configs.is_empty() {
            return Ok(skipped_ai_selection("AI configuration is unavailable"));
        }
        let effective_retries = bounded_retry_attempts(self.retry_attempts.max(
            bounded_retry_attempts_from_i64(subscriber.ai_retry_attempts),
        ));
        let mut last_error = String::new();
        for ai_config in ai_configs {
            let mut client =
                match self
                    .factory
                    .build_client(&ai_config, effective_retries, defaults.temperature)
                {
                    Ok(client) => client,
                    Err(error) => {
                        last_error = error;
                        continue;
                    }
                };
            match select_articles_with_retries(
                client.as_mut(),
                subscriber,
                defaults,
                candidates_for_model,
                candidates_by_id,
                delivery_dedupe,
                self.max_rounds,
            ) {
                Ok(selection_result) => {
                    let accepted = apply_selection_rules(
                        &selection_result,
                        subscriber,
                        candidates_by_id,
                        delivery_dedupe,
                    );
                    let mut final_summary = selection_result.summary;
                    if !accepted.is_empty() {
                        let selected_candidates = selected_candidates(&accepted, candidates_by_id);
                        if !selected_candidates.is_empty() {
                            if let Ok(summary) =
                                client.summarize_selected_articles(subscriber, &selected_candidates)
                            {
                                if !summary.trim().is_empty() {
                                    final_summary = summary;
                                }
                            }
                        }
                    }
                    return Ok(AiSelectionOutcome {
                        accepted,
                        summary: final_summary,
                        skip_reason: None,
                    });
                }
                Err(error) => {
                    last_error = error;
                }
            }
        }
        Ok(skipped_ai_selection(&format!(
            "AI selection failed across configured endpoints: {last_error}"
        )))
    }
}

pub(super) struct LiveAiSelectionClientFactory {
    timeout_seconds: u64,
}

impl AiSelectionClientFactory for LiveAiSelectionClientFactory {
    fn build_client(
        &mut self,
        config: &AiRuntimeConfig,
        retry_attempts: usize,
        temperature: f64,
    ) -> Result<Box<dyn AiSelectionClient>, String> {
        let client = live_ai_client(self.timeout_seconds, retry_attempts, temperature)
            .map_err(|error| error.to_string())?;
        Ok(Box::new(LiveAiSelectionClient {
            config: config.clone(),
            client,
        }))
    }
}

struct LiveAiSelectionClient {
    config: AiRuntimeConfig,
    client: AiCompletionClient<ReqwestAiTransport>,
}

impl AiSelectionClient for LiveAiSelectionClient {
    fn select_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        defaults: &NotificationDefaults,
        candidates: &[ArticleCandidateInfo],
    ) -> Result<SelectionResultInfo, String> {
        self.client
            .select_articles(&self.config, subscriber, defaults, candidates)
            .map_err(ai_client_error)
    }

    fn summarize_selected_articles(
        &mut self,
        subscriber: &NotificationSubscriberInfo,
        selected_candidates: &[ArticleCandidateInfo],
    ) -> Result<String, String> {
        self.client
            .summarize_selected_articles(&self.config, subscriber, selected_candidates)
            .map_err(ai_client_error)
    }
}

fn ai_client_error(error: AiClientError) -> String {
    error.to_string()
}

fn skipped_ai_selection(reason: &str) -> AiSelectionOutcome {
    AiSelectionOutcome {
        accepted: Vec::new(),
        summary: String::new(),
        skip_reason: Some(reason.to_string()),
    }
}

fn select_articles_with_retries(
    client: &mut dyn AiSelectionClient,
    subscriber: &NotificationSubscriberInfo,
    defaults: &NotificationDefaults,
    candidates_for_model: &[ArticleCandidateInfo],
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
    delivery_dedupe: &BTreeMap<String, String>,
    max_rounds: usize,
) -> Result<SelectionResultInfo, String> {
    let mut remaining_candidates = candidates_for_model.to_vec();
    let mut aggregated = BTreeMap::<i64, RankedSelectionInfo>::new();
    let mut summary = String::new();
    for _ in 0..max_rounds.max(1) {
        if remaining_candidates.is_empty() {
            break;
        }
        let round_result = client.select_articles(subscriber, defaults, &remaining_candidates)?;
        if summary.is_empty() && !round_result.summary.trim().is_empty() {
            summary = round_result.summary;
        }
        for selection in round_result.selections {
            match aggregated.get(&selection.article_id) {
                Some(existing) if existing.score >= selection.score => {}
                _ => {
                    aggregated.insert(selection.article_id, selection);
                }
            }
        }
        let merged = merged_selection_result(&summary, &aggregated);
        let accepted =
            apply_selection_rules(&merged, subscriber, candidates_by_id, delivery_dedupe);
        if accepted.len() >= MAX_ARTICLES_PER_PUSH {
            return Ok(merged);
        }
        let selected_ids = aggregated.keys().copied().collect::<BTreeSet<_>>();
        remaining_candidates.retain(|candidate| !selected_ids.contains(&candidate.article_id));
    }
    Ok(merged_selection_result(&summary, &aggregated))
}

fn merged_selection_result(
    summary: &str,
    aggregated: &BTreeMap<i64, RankedSelectionInfo>,
) -> SelectionResultInfo {
    let mut selections = aggregated.values().copied().collect::<Vec<_>>();
    selections.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    SelectionResultInfo {
        summary: summary.to_string(),
        selections,
    }
}

fn selected_candidates(
    accepted: &[RankedSelectionInfo],
    candidates_by_id: &BTreeMap<i64, ArticleCandidateInfo>,
) -> Vec<ArticleCandidateInfo> {
    accepted
        .iter()
        .filter_map(|selection| candidates_by_id.get(&selection.article_id).cloned())
        .collect()
}
pub(super) fn candidates_by_id(
    candidates: &[ArticleCandidateInfo],
) -> BTreeMap<i64, ArticleCandidateInfo> {
    candidates
        .iter()
        .map(|candidate| (candidate.article_id, candidate.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn default_ai_selector_falls_back_to_backup_endpoint() {
        let builds = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let factory = ScriptedAiFactory::new(
            vec![
                ScriptedAiClient::new(
                    vec![Err("primary unavailable".to_string())],
                    Vec::new(),
                    None,
                ),
                ScriptedAiClient::new(
                    vec![Ok(selection_result(&[102], "backup"))],
                    vec![Ok("backup summary".to_string())],
                    None,
                ),
            ],
            builds.clone(),
        );
        let mut selector = DefaultDeliveryAiSelector::new(factory, 1, 5);
        let subscriber = subscriber_info_with_backup();
        let candidates = vec![candidate_info(102)];
        let candidates_by_id = candidates_by_id(&candidates);
        let outcome = selector
            .select_for_subscriber(DeliveryAiSelectionRequest {
                subscriber: &subscriber,
                global_config: &global_config(),
                defaults: &defaults(),
                override_model: None,
                candidates_for_model: &candidates,
                candidates_by_id: &candidates_by_id,
                delivery_dedupe: &BTreeMap::new(),
            })
            .expect("AI selection should succeed through backup");

        assert_eq!(outcome.accepted[0].article_id, 102);
        assert_eq!(outcome.summary, "backup summary");
        assert_eq!(
            builds
                .borrow()
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            vec!["https://primary.test/v1", "https://backup.test/v1"]
        );
    }

    #[test]
    fn oversized_retry_counts_are_bounded_before_client_construction() {
        let builds = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let factory = ScriptedAiFactory::new(
            vec![ScriptedAiClient::new(
                vec![Ok(selection_result(&[101], "bounded"))],
                Vec::new(),
                None,
            )],
            builds.clone(),
        );
        let mut selector = DefaultDeliveryAiSelector::new(factory, usize::MAX, 5);
        let subscriber = NotificationSubscriberInfo {
            ai_retry_attempts: i64::MAX,
            ..subscriber_info()
        };
        let candidates = vec![candidate_info(101)];
        let candidates_by_id = candidates_by_id(&candidates);

        selector
            .select_for_subscriber(DeliveryAiSelectionRequest {
                subscriber: &subscriber,
                global_config: &global_config(),
                defaults: &defaults(),
                override_model: None,
                candidates_for_model: &candidates,
                candidates_by_id: &candidates_by_id,
                delivery_dedupe: &BTreeMap::new(),
            })
            .expect("AI selection should use a bounded retry count");

        assert_eq!(
            builds.borrow()[0].1,
            litradar_domain::DELIVERY_RETRY_ATTEMPTS_MAX
        );
    }

    #[test]
    fn default_ai_selector_queries_remaining_candidates_across_rounds() {
        let batch_sizes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let factory = ScriptedAiFactory::new(
            vec![ScriptedAiClient::new(
                vec![
                    Ok(selection_result(&[101], "round one")),
                    Ok(selection_result(&[102], "")),
                ],
                Vec::new(),
                Some(batch_sizes.clone()),
            )],
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
        );
        let mut selector = DefaultDeliveryAiSelector::new(factory, 1, 5);
        let subscriber = subscriber_info();
        let candidates = vec![candidate_info(101), candidate_info(102)];
        let candidates_by_id = candidates_by_id(&candidates);
        let outcome = selector
            .select_for_subscriber(DeliveryAiSelectionRequest {
                subscriber: &subscriber,
                global_config: &global_config(),
                defaults: &defaults(),
                override_model: None,
                candidates_for_model: &candidates,
                candidates_by_id: &candidates_by_id,
                delivery_dedupe: &BTreeMap::new(),
            })
            .expect("AI selection should aggregate rounds");

        assert_eq!(
            outcome
                .accepted
                .iter()
                .map(|selection| selection.article_id)
                .collect::<Vec<_>>(),
            vec![101, 102]
        );
        assert_eq!(*batch_sizes.borrow(), vec![2, 1]);
    }
    struct ScriptedAiFactory {
        clients: Vec<ScriptedAiClient>,
        builds: std::rc::Rc<std::cell::RefCell<Vec<(String, usize)>>>,
    }

    impl ScriptedAiFactory {
        fn new(
            clients: Vec<ScriptedAiClient>,
            builds: std::rc::Rc<std::cell::RefCell<Vec<(String, usize)>>>,
        ) -> Self {
            Self {
                clients: clients.into_iter().rev().collect(),
                builds,
            }
        }
    }

    impl AiSelectionClientFactory for ScriptedAiFactory {
        fn build_client(
            &mut self,
            config: &AiRuntimeConfig,
            retry_attempts: usize,
            _temperature: f64,
        ) -> Result<Box<dyn AiSelectionClient>, String> {
            self.builds
                .borrow_mut()
                .push((config.base_url.clone(), retry_attempts));
            self.clients
                .pop()
                .map(|client| Box::new(client) as Box<dyn AiSelectionClient>)
                .ok_or_else(|| "missing scripted AI client".to_string())
        }
    }

    struct ScriptedAiClient {
        selections: Vec<Result<SelectionResultInfo, String>>,
        summaries: Vec<Result<String, String>>,
        batch_sizes: Option<std::rc::Rc<std::cell::RefCell<Vec<usize>>>>,
    }

    impl ScriptedAiClient {
        fn new(
            selections: Vec<Result<SelectionResultInfo, String>>,
            summaries: Vec<Result<String, String>>,
            batch_sizes: Option<std::rc::Rc<std::cell::RefCell<Vec<usize>>>>,
        ) -> Self {
            Self {
                selections: selections.into_iter().rev().collect(),
                summaries: summaries.into_iter().rev().collect(),
                batch_sizes,
            }
        }
    }

    impl AiSelectionClient for ScriptedAiClient {
        fn select_articles(
            &mut self,
            _subscriber: &NotificationSubscriberInfo,
            _defaults: &NotificationDefaults,
            candidates: &[ArticleCandidateInfo],
        ) -> Result<SelectionResultInfo, String> {
            if let Some(batch_sizes) = &self.batch_sizes {
                batch_sizes.borrow_mut().push(candidates.len());
            }
            self.selections
                .pop()
                .unwrap_or_else(|| Err("missing scripted selection".to_string()))
        }

        fn summarize_selected_articles(
            &mut self,
            _subscriber: &NotificationSubscriberInfo,
            _selected_candidates: &[ArticleCandidateInfo],
        ) -> Result<String, String> {
            self.summaries.pop().unwrap_or_else(|| Ok(String::new()))
        }
    }
    fn selection_result(article_ids: &[i64], summary: &str) -> SelectionResultInfo {
        SelectionResultInfo {
            summary: summary.to_string(),
            selections: article_ids
                .iter()
                .enumerate()
                .map(|(index, article_id)| RankedSelectionInfo {
                    article_id: *article_id,
                    score: 100.0 - index as f64,
                })
                .collect(),
        }
    }

    fn global_config() -> NotificationGlobalConfig {
        NotificationGlobalConfig {
            ai_base_url: "https://primary.test/v1".to_string(),
            ai_api_key: "global-key".to_string(),
            pushplus_channel: "wechat".to_string(),
            pushplus_template: "markdown".to_string(),
            pushplus_topic: None,
            pushplus_option: None,
            ai_system_prompt: None,
        }
    }

    fn defaults() -> NotificationDefaults {
        NotificationDefaults {
            max_candidates: 120,
            ai_model: "model".to_string(),
            temperature: 0.2,
        }
    }

    fn subscriber_info() -> NotificationSubscriberInfo {
        NotificationSubscriberInfo {
            subscriber_id: "1".to_string(),
            user_id: 1,
            name: "Alice".to_string(),
            pushplus_token: "token".to_string(),
            channel: Some("wechat".to_string()),
            keywords: vec!["rust".to_string()],
            directions: vec!["systems".to_string()],
            selected_databases: Vec::new(),
            topic: None,
            template: Some("markdown".to_string()),
            delivery_method: "pushplus".to_string(),
            tracking_folder_id: Some(1),
            sync_to_tracking_folder: true,
            ai_base_url: Some("https://primary.test/v1".to_string()),
            ai_api_key: Some("primary-key".to_string()),
            ai_model: Some("model".to_string()),
            ai_system_prompt: None,
            ai_backup_base_url: None,
            ai_backup_api_key: None,
            ai_backup_model: None,
            ai_backup_system_prompt: None,
            ai_retry_attempts: 1,
        }
    }

    fn subscriber_info_with_backup() -> NotificationSubscriberInfo {
        NotificationSubscriberInfo {
            ai_backup_base_url: Some("https://backup.test/v1".to_string()),
            ai_backup_api_key: Some("backup-key".to_string()),
            ..subscriber_info()
        }
    }

    fn candidate_info(article_id: i64) -> ArticleCandidateInfo {
        ArticleCandidateInfo {
            article_id,
            journal_id: 1,
            issue_id: Some(11),
            title: format!("Rust systems {article_id}"),
            abstract_text: "rust systems".to_string(),
            date: Some("2026-07-01".to_string()),
            journal_title: "Fixture Journal".to_string(),
            doi: Some(format!("10.0000/{article_id}")),
            full_text_file: None,
            permalink: Some(format!("https://example.test/{article_id}")),
            open_access: true,
            in_press: false,
            within_library_holdings: true,
        }
    }
}
