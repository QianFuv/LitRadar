//! PushPlus message construction and delivery.

use super::*;

pub(super) trait DeliveryPushPlusSender {
    fn send(&mut self, message: &PushPlusMessage) -> Result<String, DeliveryError>;
}

pub(super) struct LiveDeliveryPushPlusSender {
    client: PushPlusClient<ReqwestPushPlusTransport>,
}

impl LiveDeliveryPushPlusSender {
    pub(super) fn new(
        timeout_seconds: u64,
        retry_attempts: usize,
        execution_control: Option<DeliveryExecutionControl>,
    ) -> Result<Self, DeliveryError> {
        Ok(Self {
            client: crate::pushplus::live_pushplus_client_with_control(
                timeout_seconds,
                retry_attempts,
                execution_control,
            )?,
        })
    }
}

impl DeliveryPushPlusSender for LiveDeliveryPushPlusSender {
    fn send(&mut self, message: &PushPlusMessage) -> Result<String, DeliveryError> {
        self.client.send(message).map_err(DeliveryError::from)
    }
}
pub(super) fn pushplus_message(
    subscriber: &NotificationSubscriberInfo,
    global_config: &NotificationGlobalConfig,
    title: &str,
    content: &str,
) -> PushPlusMessage {
    PushPlusMessage {
        token: subscriber.pushplus_token.clone(),
        title: title.to_string(),
        content: content.to_string(),
        channel: subscriber
            .channel
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(global_config.pushplus_channel.as_str())
            .to_string(),
        template: subscriber
            .template
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(global_config.pushplus_template.as_str())
            .to_string(),
        topic: subscriber
            .topic
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| global_config.pushplus_topic.clone()),
        option: global_config.pushplus_option.clone(),
        to: None,
    }
}
