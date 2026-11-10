-- name: NotificationSettings :one
SELECT id,user_id,keywords,directions,selected_databases,delivery_method,pushplus_token,pushplus_template,pushplus_topic,pushplus_channel,sync_to_tracking_folder,ai_base_url,ai_api_key,ai_model,ai_system_prompt,ai_backup_base_url,ai_backup_api_key,ai_backup_model,ai_backup_system_prompt,ai_retry_attempts,enabled,created_at,updated_at FROM notification_settings WHERE user_id=?;

-- name: Subscribers :many
SELECT ns.user_id,u.username,ns.keywords,ns.directions,ns.selected_databases,
ns.delivery_method,ns.pushplus_token,ns.pushplus_template,ns.pushplus_topic,ns.pushplus_channel,
ns.sync_to_tracking_folder,ns.ai_base_url,ns.ai_api_key,ns.ai_model,ns.ai_system_prompt,
ns.ai_backup_base_url,ns.ai_backup_api_key,ns.ai_backup_model,ns.ai_backup_system_prompt,ns.ai_retry_attempts,
(SELECT id FROM folders f WHERE f.user_id=ns.user_id AND f.is_tracking=1 LIMIT 1)
FROM notification_settings ns JOIN users u ON u.id=ns.user_id WHERE ns.enabled=1 ORDER BY ns.user_id;

-- name: Subscriber :one
SELECT ns.user_id,u.username,ns.keywords,ns.directions,ns.selected_databases,
ns.delivery_method,ns.pushplus_token,ns.pushplus_template,ns.pushplus_topic,ns.pushplus_channel,
ns.sync_to_tracking_folder,ns.ai_base_url,ns.ai_api_key,ns.ai_model,ns.ai_system_prompt,
ns.ai_backup_base_url,ns.ai_backup_api_key,ns.ai_backup_model,ns.ai_backup_system_prompt,ns.ai_retry_attempts,
(SELECT id FROM folders f WHERE f.user_id=ns.user_id AND f.is_tracking=1 LIMIT 1)
FROM notification_settings ns JOIN users u ON u.id=ns.user_id WHERE ns.enabled=1 AND ns.user_id=?;

