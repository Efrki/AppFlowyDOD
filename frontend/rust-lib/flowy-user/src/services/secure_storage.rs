//! Stores the user's encryption secret in the OS credential store (macOS Keychain,
//! Windows Credential Manager, Linux Secret Service) instead of the plaintext KV store.
//! On platforms without a credential store the caller falls back to the KV store.

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod imp {
  use anyhow::Result;
  use keyring::Entry;

  const KEYCHAIN_SERVICE: &str = "appflowy";

  fn entry(uid: i64) -> Result<Entry> {
    Ok(Entry::new(
      KEYCHAIN_SERVICE,
      &format!("encrypt_secret:{}", uid),
    )?)
  }

  pub fn store_encrypt_secret(uid: i64, secret: &str) -> Result<()> {
    entry(uid)?.set_password(secret)?;
    Ok(())
  }

  pub fn load_encrypt_secret(uid: i64) -> Option<String> {
    entry(uid).ok()?.get_password().ok()
  }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
  use anyhow::Result;

  pub fn store_encrypt_secret(_uid: i64, _secret: &str) -> Result<()> {
    Err(anyhow::anyhow!(
      "No OS credential store available on this platform"
    ))
  }

  pub fn load_encrypt_secret(_uid: i64) -> Option<String> {
    None
  }
}

pub use imp::{load_encrypt_secret, store_encrypt_secret};
