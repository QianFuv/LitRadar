//! Tracking-folder favorite planning and persistence.

use super::*;

pub(super) fn favorite_writes(
    config: &RecommendationRunConfig,
    subscriber: &NotificationSubscriberInfo,
    selected_article_ids: &[i64],
) -> Vec<FavoriteWritePlan> {
    let should_write = match config.workflow {
        DeliveryWorkflow::Notify => subscriber.sync_to_tracking_folder,
        DeliveryWorkflow::Push => true,
    };
    if !should_write {
        return Vec::new();
    }
    let Some(folder_id) = subscriber.tracking_folder_id else {
        return Vec::new();
    };
    selected_article_ids
        .iter()
        .map(|article_id| FavoriteWritePlan {
            user_id: subscriber.user_id,
            folder_id,
            article_id: *article_id,
            db_name: config.db_name.clone(),
        })
        .collect()
}
pub(super) fn execute_favorite_writes(
    config: &RecommendationRunConfig,
    favorite_writes: &[FavoriteWritePlan],
) -> Result<(), DeliveryError> {
    let mut grouped: BTreeMap<(i64, i64), Vec<FavoriteAdd>> = BTreeMap::new();
    for write in favorite_writes {
        grouped
            .entry((write.user_id, write.folder_id))
            .or_default()
            .push(FavoriteAdd {
                article_id: litradar_domain::ArticleId(write.article_id),
                db_name: write.db_name.clone(),
                note: String::new(),
            });
    }
    for ((user_id, folder_id), articles) in grouped {
        litradar_storage::bulk_add_favorites(
            &config.auth_db_path,
            UserId(user_id),
            folder_id,
            &articles,
        )?;
    }
    Ok(())
}
