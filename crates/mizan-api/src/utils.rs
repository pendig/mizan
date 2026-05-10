use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, aead::KeyInit};
use axum::{Json, http::StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD};
use mizan_core::{AppError, AppResult, DatabaseBackend, ErrorEnvelope};
use sha2::{Digest, Sha256};

const NONCE_BYTES: usize = 12;
const MIN_ENCRYPTED_BYTES: usize = NONCE_BYTES + 16;
const SECRET_CONTEXT: &str = "provider-connection-secret-v1";

pub fn from_app_error(error: AppError) -> (StatusCode, Json<ErrorEnvelope>) {
    let status = match error {
        AppError::InvalidConfig { .. } => StatusCode::BAD_REQUEST,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        AppError::Provider(_) => StatusCode::BAD_GATEWAY,
        AppError::LimitExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        AppError::InsufficientCredit => StatusCode::PAYMENT_REQUIRED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (status, Json(ErrorEnvelope::from(&error)))
}

pub fn prepare_sql(database_backend: DatabaseBackend, query: &'_ str) -> String {
    match database_backend {
        DatabaseBackend::Sqlite => query.to_string(),
        DatabaseBackend::Postgres => to_dollar_params(query),
    }
}

pub fn is_enabled(raw: i64) -> bool {
    raw != 0
}

pub fn parse_timestamp(raw: &str) -> AppResult<i64> {
    raw.parse::<i64>()
        .map_err(|error| AppError::infrastructure(format!("invalid timestamp: {error}")))
}

pub fn unix_timestamp_string() -> String {
    now_utc_epoch_seconds().to_string()
}

pub fn now_utc_epoch_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

pub fn is_unique_constraint_error(message: &str) -> bool {
    let normalized = message.to_lowercase();
    normalized.contains("unique")
        && (normalized.contains("constraint") || normalized.contains("already exists"))
}

pub fn encrypt_provider_api_key(
    provider_secret: &str,
    provider_connection_id: &str,
    api_key: &str,
) -> AppResult<String> {
    if provider_secret.is_empty() {
        return Err(AppError::invalid_config(
            "MIZAN_PROVIDER_SECRET_KEY",
            "provider secret key is required",
        ));
    }

    if provider_connection_id.trim().is_empty() {
        return Err(AppError::invalid_config(
            "provider_connection_id",
            "provider_connection_id is required",
        ));
    }

    if api_key.trim().is_empty() {
        return Err(AppError::invalid_config(
            "provider_connection.api_key_encrypted",
            "api_key is required",
        ));
    }

    let cipher = provider_cipher(provider_secret)?;
    let nonce = derive_nonce(provider_secret, provider_connection_id);
    let mut payload = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), api_key.as_bytes())
        .map_err(|error| {
            AppError::infrastructure(format!("provider api key encryption failed: {error}"))
        })?;
    let mut encrypted = nonce.to_vec();
    encrypted.append(&mut payload);

    Ok(STANDARD.encode(&encrypted))
}

pub(crate) fn decrypt_provider_api_key(
    provider_secret: &str,
    provider_connection_id: &str,
    encrypted_api_key: &str,
) -> AppResult<String> {
    if provider_secret.is_empty() {
        return Err(AppError::invalid_config(
            "MIZAN_PROVIDER_SECRET_KEY",
            "provider secret key is required",
        ));
    }

    if provider_connection_id.trim().is_empty() {
        return Err(AppError::invalid_config(
            "provider_connection_id",
            "provider_connection_id is required",
        ));
    }

    if encrypted_api_key.trim().is_empty() {
        return Err(AppError::invalid_config(
            "provider_connection.api_key_encrypted",
            "encrypted api key is required",
        ));
    }

    let data = STANDARD.decode(encrypted_api_key).map_err(|error| {
        AppError::invalid_config("provider_connection.api_key_encrypted", error.to_string())
    })?;

    if data.len() < MIN_ENCRYPTED_BYTES {
        return Err(AppError::invalid_config(
            "provider_connection.api_key_encrypted",
            "encrypted api key is invalid",
        ));
    }

    let nonce = aes_gcm::Nonce::<aes_gcm::aead::generic_array::typenum::U12>::from_slice(
        &data[..NONCE_BYTES],
    );
    let cipher = provider_cipher(provider_secret)?;
    let plaintext = cipher
        .decrypt(nonce, &data[NONCE_BYTES..])
        .map_err(|error| {
            AppError::infrastructure(format!("provider api key decryption failed: {error}"))
        })?;

    String::from_utf8(plaintext).map_err(|error| {
        AppError::invalid_config(
            "provider_connection.api_key_encrypted",
            format!("invalid stored key format: {error}"),
        )
    })
}

fn derive_nonce(provider_secret: &str, provider_connection_id: &str) -> [u8; NONCE_BYTES] {
    let material = Sha256::digest(format!(
        "{SECRET_CONTEXT}:{provider_secret}:{provider_connection_id}"
    ));
    let mut bytes = [0u8; NONCE_BYTES];
    bytes.copy_from_slice(&material[..NONCE_BYTES]);
    bytes
}

fn provider_cipher(provider_secret: &str) -> AppResult<Aes256Gcm> {
    let material = Sha256::digest(provider_secret.as_bytes());
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&material);
    Ok(Aes256Gcm::new(key))
}

fn to_dollar_params(query: &str) -> String {
    let mut parameter_index = 0usize;
    let mut converted = String::with_capacity(query.len());
    let mut chars = query.chars().peekable();

    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while let Some(current) = chars.next() {
        if in_line_comment {
            if current == '\n' {
                in_line_comment = false;
            }
            converted.push(current);
            continue;
        }

        if in_block_comment {
            if current == '*' && chars.peek() == Some(&'/') {
                converted.push(current);
                converted.push(chars.next().expect("peeked block comment terminator"));
                in_block_comment = false;
            } else {
                converted.push(current);
            }
            continue;
        }

        if in_single_quote {
            if current == '\'' {
                if chars.peek() == Some(&'\'') {
                    converted.push(current);
                    converted.push(chars.next().expect("peeked escaped single quote"));
                    continue;
                }

                in_single_quote = false;
            }
            converted.push(current);
            continue;
        }

        if in_double_quote {
            if current == '"' {
                if chars.peek() == Some(&'"') {
                    converted.push(current);
                    converted.push(chars.next().expect("peeked escaped double quote"));
                    continue;
                }

                in_double_quote = false;
            }
            converted.push(current);
            continue;
        }

        if current == '-' && chars.peek() == Some(&'-') {
            in_line_comment = true;
            converted.push(current);
            converted.push(chars.next().expect("peeked line comment"));
            continue;
        }

        if current == '/' && chars.peek() == Some(&'*') {
            in_block_comment = true;
            converted.push(current);
            converted.push(chars.next().expect("peeked block comment"));
            continue;
        }

        if current == '\'' {
            in_single_quote = true;
            converted.push(current);
            continue;
        }

        if current == '"' {
            in_double_quote = true;
            converted.push(current);
            continue;
        }

        if current == '?' {
            parameter_index += 1;
            converted.push('$');
            converted.push_str(&parameter_index.to_string());
            continue;
        }

        converted.push(current);
    }

    converted
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn prepare_sql_keeps_question_marks_for_sqlite() {
        let prepared = prepare_sql(DatabaseBackend::Sqlite, "SELECT * FROM x WHERE id = ?");
        assert_eq!(prepared, "SELECT * FROM x WHERE id = ?");
    }

    #[test]
    fn prepare_sql_converts_question_marks_for_postgres() {
        let prepared = prepare_sql(
            DatabaseBackend::Postgres,
            "SELECT * FROM x WHERE a = ? AND b = ?",
        );
        assert_eq!(prepared, "SELECT * FROM x WHERE a = $1 AND b = $2");
    }

    #[test]
    fn to_dollar_params_keeps_question_mark_in_quoted_string() {
        let prepared = prepare_sql(
            DatabaseBackend::Postgres,
            "SELECT * FROM x WHERE note = 'Is this a question?' OR name = ?",
        );
        assert_eq!(
            prepared,
            "SELECT * FROM x WHERE note = 'Is this a question?' OR name = $1",
        );
    }

    #[test]
    fn to_dollar_params_keeps_question_mark_in_comment() {
        let prepared = prepare_sql(
            DatabaseBackend::Postgres,
            "SELECT * FROM x WHERE enabled = 1 -- ? should stay here\nAND id = ?",
        );
        assert_eq!(
            prepared,
            "SELECT * FROM x WHERE enabled = 1 -- ? should stay here\nAND id = $1",
        );
    }

    #[test]
    fn provider_api_key_can_encrypt_and_decrypt() {
        let provider_id = Uuid::now_v7().to_string();
        let provider_secret = "phase-1-secret-key";
        let original = "sk-live-abc";

        let encrypted = encrypt_provider_api_key(provider_secret, &provider_id, original)
            .expect("encrypt provider key");
        assert_ne!(encrypted, original);

        let decrypted = decrypt_provider_api_key(provider_secret, &provider_id, &encrypted)
            .expect("decrypt provider key");
        assert_eq!(decrypted, original);
    }

    #[test]
    fn provider_api_key_encryption_fails_without_secret() {
        let provider_id = Uuid::now_v7().to_string();
        let result = encrypt_provider_api_key("", &provider_id, "sk-live-abc");
        assert!(result.is_err());
    }

    #[test]
    fn provider_api_key_encryption_roundtrips_across_provider_ids() {
        let provider_secret = "phase-1-secret-key";
        let original = "sk-live-abc";

        let id_a = Uuid::now_v7().to_string();
        let id_b = Uuid::now_v7().to_string();
        let encrypted_a = encrypt_provider_api_key(provider_secret, &id_a, original)
            .expect("encrypt provider key a");
        let encrypted_b = encrypt_provider_api_key(provider_secret, &id_b, original)
            .expect("encrypt provider key b");

        assert_ne!(encrypted_a, encrypted_b);
    }
}
