use std::sync::Arc;

use flowy_error::FlowyResult;
use flowy_sqlite::kv::KVStorePreferences;
use flowy_user_pub::cloud::UserCloudConfig;
use lib_infra::encryption::generate_encryption_secret;

use crate::services::secure_storage;

const CLOUD_CONFIG_KEY: &str = "af_user_cloud_config";

/// Persist the config. The encryption secret goes to the OS credential store when
/// one is available; the KV copy is then scrubbed. On platforms without a
/// credential store (or when it fails) the secret stays in the KV store as before.
fn persist_cloud_config(
  uid: i64,
  store_preference: &Arc<KVStorePreferences>,
  config: &UserCloudConfig,
) -> FlowyResult<()> {
  let mut stored = config.clone();
  if !stored.encrypt_secret.is_empty() {
    match secure_storage::store_encrypt_secret(uid, &stored.encrypt_secret) {
      Ok(()) => stored.encrypt_secret = String::new(),
      Err(err) => tracing::warn!(
        "OS credential store unavailable, keeping encrypt secret in KV store: {}",
        err
      ),
    }
  }
  let key = cache_key_for_cloud_config(uid);
  store_preference.set_object(&key, &stored)?;
  Ok(())
}

fn generate_cloud_config(uid: i64, store_preference: &Arc<KVStorePreferences>) -> UserCloudConfig {
  let config = UserCloudConfig::new(generate_encryption_secret());
  if let Err(err) = persist_cloud_config(uid, store_preference, &config) {
    tracing::error!("Failed to persist cloud config: {}", err);
  }
  config
}

pub fn save_cloud_config(
  uid: i64,
  store_preference: &Arc<KVStorePreferences>,
  config: &UserCloudConfig,
) -> FlowyResult<()> {
  tracing::info!("save user:{} cloud config: {}", uid, config);
  persist_cloud_config(uid, store_preference, config)
}

fn cache_key_for_cloud_config(uid: i64) -> String {
  format!("{}:{}", CLOUD_CONFIG_KEY, uid)
}

pub fn get_cloud_config(
  uid: i64,
  store_preference: &Arc<KVStorePreferences>,
) -> Option<UserCloudConfig> {
  let key = cache_key_for_cloud_config(uid);
  let mut config = store_preference.get_object::<UserCloudConfig>(&key)?;
  if config.encrypt_secret.is_empty() {
    if let Some(secret) = secure_storage::load_encrypt_secret(uid) {
      config.encrypt_secret = secret;
    }
  } else {
    // The secret predates credential-store support: migrate it out of the KV store.
    if secure_storage::store_encrypt_secret(uid, &config.encrypt_secret).is_ok() {
      let mut scrubbed = config.clone();
      scrubbed.encrypt_secret = String::new();
      let _ = store_preference.set_object(&key, &scrubbed);
    }
  }
  Some(config)
}

pub fn get_or_create_cloud_config(
  uid: i64,
  store_preferences: &Arc<KVStorePreferences>,
) -> UserCloudConfig {
  get_cloud_config(uid, store_preferences)
    .unwrap_or_else(|| generate_cloud_config(uid, store_preferences))
}

pub fn get_encrypt_secret(uid: i64, store_preference: &Arc<KVStorePreferences>) -> Option<String> {
  get_cloud_config(uid, store_preference).map(|config| config.encrypt_secret)
}
