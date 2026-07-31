use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::Result;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use pbkdf2::hmac::Hmac;
use pbkdf2::pbkdf2;
use rand::distributions::Alphanumeric;
use rand::rngs::OsRng;
use rand::Rng;
use sha2::Sha256;

/// The length of the salt in bytes.
const SALT_LENGTH: usize = 16;

/// The length of the derived encryption key in bytes.
const KEY_LENGTH: usize = 32;

/// The number of iterations for the legacy PBKDF2 key derivation.
const LEGACY_PBKDF2_ITERATIONS: u32 = 1000;

/// The length of the nonce for legacy AES-GCM encryption.
const LEGACY_NONCE_LENGTH: usize = 12;

/// The length of the nonce for XChaCha20-Poly1305 encryption.
const XNONCE_LENGTH: usize = 24;

/// Argon2id memory cost in KiB (19 MiB, OWASP recommendation).
const ARGON2_MEMORY_KIB: u32 = 19 * 1024;

/// Argon2id iteration count (OWASP recommendation).
const ARGON2_ITERATIONS: u32 = 2;

/// Argon2id lanes.
const ARGON2_PARALLELISM: u32 = 1;

/// Delimiter used to concatenate the secret parts.
const CONCATENATED_DELIMITER: &str = "$";

/// Version tag of secrets whose data is protected with Argon2id + XChaCha20-Poly1305.
const SECRET_VERSION_2: &str = "v2";

/// The length of the random passphrase in characters.
const PASSPHRASE_LENGTH: usize = 30;

enum Secret<'a> {
  /// `<passphrase>$<salt_b64>` — PBKDF2-HMAC-SHA256 + AES-256-GCM.
  Legacy {
    passphrase: &'a str,
    salt: [u8; SALT_LENGTH],
  },
  /// `v2$<passphrase>$<salt_b64>` — Argon2id + XChaCha20-Poly1305.
  V2 {
    passphrase: &'a str,
    salt: [u8; SALT_LENGTH],
  },
}

/// Generate a new encryption secret consisting of a version tag, a passphrase and a salt.
pub fn generate_encryption_secret() -> String {
  let passphrase = generate_random_passphrase();
  let salt = generate_random_salt();
  format!(
    "{}{}{}{}{}",
    SECRET_VERSION_2,
    CONCATENATED_DELIMITER,
    passphrase,
    CONCATENATED_DELIMITER,
    STANDARD.encode(salt)
  )
}

/// Encrypt a byte slice using the scheme selected by the secret version.
///
/// # Arguments
/// * `data`: The data to encrypt.
/// * `combined_passphrase_salt`: The versioned secret string.
pub fn encrypt_data<T: AsRef<[u8]>>(data: T, combined_passphrase_salt: &str) -> Result<Vec<u8>> {
  match parse_secret(combined_passphrase_salt)? {
    Secret::V2 { passphrase, salt } => {
      let key = derive_key_argon2id(passphrase, &salt)?;
      let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(&key));
      let nonce: [u8; XNONCE_LENGTH] = OsRng.gen();
      let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), data.as_ref())
        .map_err(|e| anyhow::anyhow!("Encryption error: {:?}", e))?;
      Ok(nonce.iter().copied().chain(ciphertext).collect())
    },
    Secret::Legacy { passphrase, salt } => {
      let key = derive_key_pbkdf2(passphrase, &salt)?;
      let cipher = Aes256Gcm::new(GenericArray::from_slice(&key));
      let nonce: [u8; LEGACY_NONCE_LENGTH] = OsRng.gen();
      let ciphertext = cipher
        .encrypt(GenericArray::from_slice(&nonce), data.as_ref())
        .map_err(|e| anyhow::anyhow!("Encryption error: {:?}", e))?;
      Ok(nonce.iter().copied().chain(ciphertext).collect())
    },
  }
}

/// Decrypt a byte slice using the scheme selected by the secret version.
///
/// # Arguments
/// * `data`: The data to decrypt.
/// * `combined_passphrase_salt`: The versioned secret string.
pub fn decrypt_data<T: AsRef<[u8]>>(data: T, combined_passphrase_salt: &str) -> Result<Vec<u8>> {
  match parse_secret(combined_passphrase_salt)? {
    Secret::V2 { passphrase, salt } => {
      if data.as_ref().len() <= XNONCE_LENGTH {
        return Err(anyhow::anyhow!("Ciphertext too short to include nonce."));
      }
      let key = derive_key_argon2id(passphrase, &salt)?;
      let cipher = XChaCha20Poly1305::new(GenericArray::from_slice(&key));
      let (nonce, cipher_data) = data.as_ref().split_at(XNONCE_LENGTH);
      cipher
        .decrypt(XNonce::from_slice(nonce), cipher_data)
        .map_err(|e| anyhow::anyhow!("Decryption error: {:?}", e))
    },
    Secret::Legacy { passphrase, salt } => {
      if data.as_ref().len() <= LEGACY_NONCE_LENGTH {
        return Err(anyhow::anyhow!("Ciphertext too short to include nonce."));
      }
      let key = derive_key_pbkdf2(passphrase, &salt)?;
      let cipher = Aes256Gcm::new(GenericArray::from_slice(&key));
      let (nonce, cipher_data) = data.as_ref().split_at(LEGACY_NONCE_LENGTH);
      cipher
        .decrypt(GenericArray::from_slice(nonce), cipher_data)
        .map_err(|e| anyhow::anyhow!("Decryption error: {:?}", e))
    },
  }
}

/// Encrypt a string and return the result as a base64 encoded string.
///
/// # Arguments
/// * `data`: The string data to encrypt.
/// * `combined_passphrase_salt`: The versioned secret string.
pub fn encrypt_text<T: AsRef<[u8]>>(data: T, combined_passphrase_salt: &str) -> Result<String> {
  let encrypted = encrypt_data(data.as_ref(), combined_passphrase_salt)?;
  Ok(STANDARD.encode(encrypted))
}

/// Decrypt a base64 encoded string.
///
/// # Arguments
/// * `data`: The base64 encoded string to decrypt.
/// * `combined_passphrase_salt`: The versioned secret string.
pub fn decrypt_text<T: AsRef<[u8]>>(data: T, combined_passphrase_salt: &str) -> Result<String> {
  let encrypted = STANDARD.decode(data)?;
  let decrypted = decrypt_data(encrypted, combined_passphrase_salt)?;
  Ok(String::from_utf8(decrypted)?)
}

/// Generates a random passphrase consisting of alphanumeric characters.
///
/// This function creates a passphrase with both uppercase and lowercase letters
/// as well as numbers. The passphrase is 30 characters in length.
///
/// # Returns
///
/// A `String` representing the generated passphrase.
///
/// # Security Considerations
///
///   The passphrase is derived from the `Alphanumeric` character set which includes 62 possible
///   characters (26 lowercase letters, 26 uppercase letters, 10 numbers). This results in a total
///   of `62^30` possible combinations, making it strong against brute force attacks.
///
fn generate_random_passphrase() -> String {
  OsRng
    .sample_iter(&Alphanumeric)
    .take(PASSPHRASE_LENGTH)
    .map(char::from)
    .collect()
}

fn generate_random_salt() -> [u8; SALT_LENGTH] {
  OsRng.gen()
}

fn parse_secret(combined: &str) -> Result<Secret<'_>> {
  let parts: Vec<&str> = combined.split(CONCATENATED_DELIMITER).collect();
  let (version, passphrase, salt_base64) = match parts.as_slice() {
    [version, passphrase, salt] if *version == SECRET_VERSION_2 => (*version, *passphrase, *salt),
    [passphrase, salt] => ("", *passphrase, *salt),
    _ => return Err(anyhow::anyhow!("Invalid combined format")),
  };
  let salt = STANDARD.decode(salt_base64)?;
  if salt.len() != SALT_LENGTH {
    return Err(anyhow::anyhow!("Incorrect salt length"));
  }
  let mut salt_array = [0u8; SALT_LENGTH];
  salt_array.copy_from_slice(&salt);
  if version == SECRET_VERSION_2 {
    Ok(Secret::V2 {
      passphrase,
      salt: salt_array,
    })
  } else {
    Ok(Secret::Legacy {
      passphrase,
      salt: salt_array,
    })
  }
}

fn derive_key_argon2id(passphrase: &str, salt: &[u8; SALT_LENGTH]) -> Result<[u8; KEY_LENGTH]> {
  let params = Params::new(
    ARGON2_MEMORY_KIB,
    ARGON2_ITERATIONS,
    ARGON2_PARALLELISM,
    Some(KEY_LENGTH),
  )
  .map_err(|e| anyhow::anyhow!("Invalid Argon2 params: {}", e))?;
  let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
  let mut key = [0u8; KEY_LENGTH];
  argon2
    .hash_password_into(passphrase.as_bytes(), salt, &mut key)
    .map_err(|e| anyhow::anyhow!("Key derivation error: {}", e))?;
  Ok(key)
}

fn derive_key_pbkdf2(passphrase: &str, salt: &[u8; SALT_LENGTH]) -> Result<[u8; KEY_LENGTH]> {
  let mut key = [0u8; KEY_LENGTH];
  pbkdf2::<Hmac<Sha256>>(
    passphrase.as_bytes(),
    salt,
    LEGACY_PBKDF2_ITERATIONS,
    &mut key,
  )?;
  Ok(key)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn generate_legacy_secret() -> String {
    let passphrase = generate_random_passphrase();
    let salt = generate_random_salt();
    format!(
      "{}{}{}",
      passphrase,
      CONCATENATED_DELIMITER,
      STANDARD.encode(salt)
    )
  }

  #[test]
  fn encrypt_decrypt_test() {
    let secret = generate_encryption_secret();
    assert!(secret.starts_with("v2$"));

    let data = b"hello world";
    let encrypted = encrypt_data(data, &secret).unwrap();
    let decrypted = decrypt_data(encrypted, &secret).unwrap();
    assert_eq!(data, decrypted.as_slice());

    let s = "123".to_string();
    let encrypted = encrypt_text(&s, &secret).unwrap();
    let decrypted_str = decrypt_text(encrypted, &secret).unwrap();
    assert_eq!(s, decrypted_str);
  }

  #[test]
  fn legacy_secret_roundtrip_test() {
    let secret = generate_legacy_secret();
    let data = b"hello legacy";
    let encrypted = encrypt_data(data, &secret).unwrap();
    let decrypted = decrypt_data(encrypted, &secret).unwrap();
    assert_eq!(data, decrypted.as_slice());
  }

  #[test]
  fn decrypt_with_invalid_secret_test() {
    let secret = generate_encryption_secret();
    let data = b"hello world";
    let encrypted = encrypt_data(data, &secret).unwrap();
    let decrypted = decrypt_data(&encrypted, "invalid secret");
    assert!(decrypted.is_err());

    let other_secret = generate_encryption_secret();
    assert!(decrypt_data(&encrypted, &other_secret).is_err());
  }

  #[test]
  fn tampered_ciphertext_test() {
    let secret = generate_encryption_secret();
    let mut encrypted = encrypt_data(b"hello world", &secret).unwrap();
    let last = encrypted.len() - 1;
    encrypted[last] ^= 0xFF;
    assert!(decrypt_data(encrypted, &secret).is_err());
  }
}
