//! Workspace- or app-scoped named secrets (`pgapp secret set/list/rm`),
//! stored in `pgapp_control.secrets` (see `db/control_schema.sql`) and
//! referenced from markup as `{{secret.<name>}}` — the same
//! interpolation `http_request` already does for page items
//! (`actions::http_request::interpolate`), just resolved from here
//! instead of `ctx.values`.
//!
//! **Encrypted, never hashed.** A hash is one-way — fine for the CLI
//! operator password in `instance.rs`, which is only ever *compared*,
//! but useless for a secret that has to be sent back out in plaintext
//! (an `Authorization` header, a query-string API key). So
//! these are AES-256-GCM encrypted at rest instead, with the key held
//! only in memory, read fresh from `PGAPP_SECRET_KEY` at process
//! start — the same "never touches disk" pattern `instance.rs` already
//! uses for the `pgapp_admin` Postgres role's own password
//! (`PGAPP_ADMIN_DB_PASSWORD`). If the key lived in this database
//! too, encrypting the secrets in it would be theater.
//!
//! An app-scoped secret shadows a workspace-scoped one of the same
//! name — the same precedent a page-scoped named query already sets
//! over an app-scoped one.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{bail, Context, Result};
use sqlx::PgPool;

/// Exactly one of these — mirrors the `secrets_exactly_one_scope`
/// check constraint.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    /// A `pgapp_control.workspaces.id`.
    Workspace(i32),
    /// A `pgapp_control.apps.id` — the control-plane app id, not
    /// `pgapp_meta.apps.id` (a different table, rewritten by every
    /// markup resync).
    App(i32),
}

/// Reads the 32-byte AES-256 key from `PGAPP_SECRET_KEY` (64 hex
/// characters) — required only by commands that actually touch a
/// secret's value (`secret set`, resolving `{{secret...}}` at request
/// time); every other command works fine without it ever being set.
pub fn load_key() -> Result<[u8; 32]> {
    let hex = std::env::var("PGAPP_SECRET_KEY").context(
        "PGAPP_SECRET_KEY is not set — it's the AES-256 key that encrypts/decrypts secrets \
         and is never stored on disk (generate one with e.g. `openssl rand -hex 32`)",
    )?;
    decode_hex_32(&hex)
}

fn decode_hex_32(hex: &str) -> Result<[u8; 32]> {
    if hex.len() != 64 {
        bail!("PGAPP_SECRET_KEY must be exactly 64 hex characters (32 bytes), got {}", hex.len());
    }
    let mut out = [0u8; 32];
    for (i, chunk) in out.iter_mut().enumerate() {
        let byte_str = &hex[i * 2..i * 2 + 2];
        *chunk = u8::from_str_radix(byte_str, 16)
            .with_context(|| format!("PGAPP_SECRET_KEY has invalid hex at position {}", i * 2))?;
    }
    Ok(out)
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
}

fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher(key)
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("failed to encrypt secret: {e}"))?;
    Ok((ciphertext, nonce.to_vec()))
}

fn decrypt(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8]) -> Result<String> {
    let plaintext = cipher(key)
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| anyhow::anyhow!("failed to decrypt secret (wrong PGAPP_SECRET_KEY?): {e}"))?;
    String::from_utf8(plaintext).context("decrypted secret is not valid UTF-8")
}

/// Creates or overwrites (by name, within its scope) one secret.
pub async fn set(pool: &PgPool, key: &[u8; 32], scope: Scope, name: &str, value: &str) -> Result<()> {
    let (ciphertext, nonce) = encrypt(key, value)?;
    match scope {
        Scope::Workspace(workspace_id) => {
            sqlx::query(
                "insert into pgapp_control.secrets (workspace_id, name, ciphertext, nonce)
                 values ($1, $2, $3, $4)
                 on conflict (workspace_id, name) where workspace_id is not null do update set
                    ciphertext = excluded.ciphertext, nonce = excluded.nonce, updated_at = now()",
            )
            .bind(workspace_id)
            .bind(name)
            .bind(&ciphertext)
            .bind(&nonce)
            .execute(pool)
            .await
            .context("failed to save workspace secret")?;
        }
        Scope::App(app_id) => {
            sqlx::query(
                "insert into pgapp_control.secrets (app_id, name, ciphertext, nonce)
                 values ($1, $2, $3, $4)
                 on conflict (app_id, name) where app_id is not null do update set
                    ciphertext = excluded.ciphertext, nonce = excluded.nonce, updated_at = now()",
            )
            .bind(app_id)
            .bind(name)
            .bind(&ciphertext)
            .bind(&nonce)
            .execute(pool)
            .await
            .context("failed to save app secret")?;
        }
    }
    Ok(())
}

/// Names only, never values — what `pgapp secret list` prints.
pub async fn list(pool: &PgPool, scope: Scope) -> Result<Vec<String>> {
    let names = match scope {
        Scope::Workspace(workspace_id) => {
            sqlx::query_scalar("select name from pgapp_control.secrets where workspace_id = $1 order by name")
                .bind(workspace_id)
                .fetch_all(pool)
                .await
        }
        Scope::App(app_id) => sqlx::query_scalar("select name from pgapp_control.secrets where app_id = $1 order by name")
            .bind(app_id)
            .fetch_all(pool)
            .await,
    }
    .context("failed to list secrets")?;
    Ok(names)
}

/// Returns whether a matching row existed.
pub async fn remove(pool: &PgPool, scope: Scope, name: &str) -> Result<bool> {
    let result = match scope {
        Scope::Workspace(workspace_id) => {
            sqlx::query("delete from pgapp_control.secrets where workspace_id = $1 and name = $2")
                .bind(workspace_id)
                .bind(name)
                .execute(pool)
                .await
        }
        Scope::App(app_id) => {
            sqlx::query("delete from pgapp_control.secrets where app_id = $1 and name = $2")
                .bind(app_id)
                .bind(name)
                .execute(pool)
                .await
        }
    }
    .context("failed to remove secret")?;
    Ok(result.rows_affected() > 0)
}

async fn fetch_scoped(pool: &PgPool, scope: Scope, name: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let row = match scope {
        Scope::Workspace(workspace_id) => {
            sqlx::query_as("select ciphertext, nonce from pgapp_control.secrets where workspace_id = $1 and name = $2")
                .bind(workspace_id)
                .bind(name)
                .fetch_optional(pool)
                .await
        }
        Scope::App(app_id) => {
            sqlx::query_as("select ciphertext, nonce from pgapp_control.secrets where app_id = $1 and name = $2")
                .bind(app_id)
                .bind(name)
                .fetch_optional(pool)
                .await
        }
    }
    .context("failed to look up secret")?;
    Ok(row)
}

/// Resolves `name` at request time: an app-scoped secret (this app's
/// own `pgapp_control.apps.id`) shadows a workspace-scoped one of the
/// same name; `Ok(None)` when neither exists (not an error — the
/// caller decides whether a missing secret is fatal).
pub async fn resolve(
    pool: &PgPool,
    key: &[u8; 32],
    control_app_id: i32,
    workspace_id: Option<i32>,
    name: &str,
) -> Result<Option<String>> {
    if let Some((ciphertext, nonce)) = fetch_scoped(pool, Scope::App(control_app_id), name).await? {
        return decrypt(key, &ciphertext, &nonce).map(Some);
    }
    if let Some(workspace_id) = workspace_id {
        if let Some((ciphertext, nonce)) = fetch_scoped(pool, Scope::Workspace(workspace_id), name).await? {
            return decrypt(key, &ciphertext, &nonce).map(Some);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let key = [7u8; 32];
        let (ciphertext, nonce) = encrypt(&key, "s3cr3t-token").unwrap();
        assert_eq!(decrypt(&key, &ciphertext, &nonce).unwrap(), "s3cr3t-token");
    }

    #[test]
    fn decrypt_fails_with_the_wrong_key() {
        let key = [7u8; 32];
        let wrong_key = [9u8; 32];
        let (ciphertext, nonce) = encrypt(&key, "s3cr3t-token").unwrap();
        assert!(decrypt(&wrong_key, &ciphertext, &nonce).is_err());
    }

    #[test]
    fn decode_hex_32_rejects_wrong_length() {
        assert!(decode_hex_32("abcd").is_err());
    }

    #[test]
    fn decode_hex_32_rejects_non_hex_characters() {
        let bad = "z".repeat(64);
        assert!(decode_hex_32(&bad).is_err());
    }

    #[test]
    fn decode_hex_32_accepts_a_valid_key() {
        let key_hex = "00".repeat(32);
        assert_eq!(decode_hex_32(&key_hex).unwrap(), [0u8; 32]);
    }
}
