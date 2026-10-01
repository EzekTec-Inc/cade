use cade_agent::agent::HttpTransport;

/// Bridge CLI environment credentials to the server using the same editable
/// protocol/endpoint definitions as the server and embedded SDK.
pub async fn push_env_providers_to_server(client: &HttpTransport) {
    for provider in cade_ai::provider_registry::ProviderRegistry::configured().get_all_providers() {
        if let Some(key) = provider.env_key() {
            let _ = client
                .add_provider(
                    &provider.name,
                    &provider.kind,
                    Some(&key),
                    Some(&provider.endpoint()),
                )
                .await;
        }
    }
}
