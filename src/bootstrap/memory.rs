use cade_agent::agent::CadeClient;

pub async fn seed_default_memory(client: &CadeClient, agent_id: &str) {
    for (label, value, description, max_chars, tier) in cade::DEFAULT_MEMORY_BLOCKS {
        if let Err(e) = client
            .upsert_memory_with_limit(agent_id, label, value, Some(description), Some(*max_chars))
            .await
        {
            tracing::warn!("seed_memory {label}: {e}");
        }
        if let Err(e) = client.set_memory_tier(agent_id, label, tier).await {
            tracing::warn!("set_memory_tier {label}={tier}: {e}");
        }
    }
}
