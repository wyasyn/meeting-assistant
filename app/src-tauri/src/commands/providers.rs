//! `list_providers`, `set_api_key`, `clear_api_key`, `test_provider` (FR-8.1).

use tauri::State;

use crate::error::AppError;
use crate::providers::{ProviderEntry, ProviderId, ProviderService, TestResult};

#[tauri::command]
pub fn list_providers(service: State<'_, ProviderService>) -> Result<Vec<ProviderEntry>, AppError> {
    service.list()
}

#[tauri::command]
pub fn set_api_key(
    service: State<'_, ProviderService>,
    provider: ProviderId,
    key: String,
) -> Result<(), AppError> {
    service.set_key(provider, &key)
}

#[tauri::command]
pub fn clear_api_key(
    service: State<'_, ProviderService>,
    provider: ProviderId,
) -> Result<(), AppError> {
    service.clear_key(provider)
}

#[tauri::command]
pub async fn test_provider(
    service: State<'_, ProviderService>,
    provider: ProviderId,
) -> Result<TestResult, AppError> {
    service.test(provider).await
}
