//! A pgapp *instance* is one target Postgres database plus a
//! dedicated `pgapp_admin` login role pgapp itself created and
//! operates as from then on — set up once via `pgapp instance init`.
//!
//! Two separate secrets are involved, deliberately kept apart:
//! - The **`pgapp_admin` Postgres role's password** is never written
//!   to disk anywhere. It's needed to actually open a connection, so a
//!   one-way hash would be useless for that — instead every command
//!   that touches an instance reads it fresh from the
//!   `PGAPP_ADMIN_DB_PASSWORD` environment variable, for that process's
//!   lifetime only.
//! - The **local CLI operator password**, set once at `instance init`,
//!   gates who's allowed to run instance/workspace/app commands against
//!   this instance at all. It's unrelated to Postgres auth, so an
//!   argon2 hash (never the plaintext) is exactly what belongs on
//!   disk — verified interactively (or via `PGAPP_CLI_ADMIN_PASSWORD`
//!   for scripts) before any of those commands proceed.
//!
//! The instance file itself (host/port/dbname/role name/password hash)
//! lives at `~/.pgapp/instances/<dbname>.json` (override the base
//! directory with `PGAPP_HOME`), `0600`, one file per instance — "one
//! pgapp instance per database instance" means the database name is
//! the natural key.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

pub const ADMIN_ROLE: &str = "pgapp_admin";

/// Pool size for connections that serve real HTTP traffic — the
/// classic-mode server pool, and the `pgapp_admin` connection
/// `pgapp run` reuses to serve an instance. Default is in the same
/// ballpark as a typical APEX/ORDS pool for one moderately busy
/// workspace: comfortably above a handful of toy connections, without
/// assuming "bigger is always faster" (a Postgres backend is a full
/// process, not a lightweight thread, so a few dozen is already
/// generous for one server). Override with `PGAPP_MAX_CONNECTIONS`.
pub fn max_connections() -> u32 {
    std::env::var("PGAPP_MAX_CONNECTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(20)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceFile {
    pub dbname: String,
    pub host: String,
    pub port: u16,
    pub admin_role: String,
    pub admin_password_hash: String,
    pub created_at: String,
}

fn instances_dir() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("PGAPP_HOME") {
        return Ok(PathBuf::from(home).join("instances"));
    }
    let home = std::env::var("HOME").context("HOME is not set and PGAPP_HOME wasn't given")?;
    Ok(PathBuf::from(home).join(".pgapp").join("instances"))
}

fn instance_path(dbname: &str) -> Result<PathBuf> {
    Ok(instances_dir()?.join(format!("{dbname}.json")))
}

/// Writes the instance file, creating `~/.pgapp/instances` (`0700`) if
/// needed and the file itself `0600` — this holds a password *hash*,
/// never a usable secret, but there's no reason to leave it world
/// readable.
pub fn save(instance: &InstanceFile) -> Result<()> {
    let dir = instances_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create '{}'", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    }

    let path = instance_path(&instance.dbname)?;
    let json = serde_json::to_string_pretty(instance).context("failed to serialize instance file")?;
    std::fs::write(&path, json).with_context(|| format!("failed to write '{}'", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to set permissions on '{}'", path.display()))?;
    }
    Ok(())
}

pub fn load(dbname: &str) -> Result<InstanceFile> {
    let path = instance_path(dbname)?;
    if !Path::new(&path).exists() {
        bail!(
            "no pgapp instance is set up for database '{dbname}' (expected '{}') — run `pgapp instance init` first",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("failed to read '{}'", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse '{}'", path.display()))
}

/// Deletes the instance file — the local half of `pgapp instance
/// destroy`; the caller is responsible for the Postgres-side teardown
/// (dropping the role/schemas) before or after this.
pub fn delete_file(dbname: &str) -> Result<()> {
    let path = instance_path(dbname)?;
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("failed to remove '{}'", path.display()))?;
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))?
        .to_string())
}

fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

fn prompt_line(label: &str) -> Result<String> {
    use std::io::Write;
    print!("{label}: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).context("failed to read from stdin")?;
    Ok(line.trim().to_string())
}

/// Gates every instance/workspace/app command behind the local CLI
/// operator password chosen at `instance init` — `PGAPP_CLI_ADMIN_PASSWORD`
/// answers non-interactively (for scripts/CI), otherwise this prompts.
pub fn verify_operator(instance: &InstanceFile) -> Result<()> {
    let entered = match std::env::var("PGAPP_CLI_ADMIN_PASSWORD") {
        Ok(v) => v,
        Err(_) => prompt_line("pgapp CLI admin password")?,
    };
    if verify_password(&entered, &instance.admin_password_hash) {
        Ok(())
    } else {
        bail!("incorrect pgapp CLI admin password for instance '{}'", instance.dbname)
    }
}

/// The `pgapp_admin` Postgres role's password — read fresh from the
/// environment on every use, never persisted. See the module doc for
/// why this can't be a hash.
pub fn admin_db_password() -> Result<String> {
    std::env::var("PGAPP_ADMIN_DB_PASSWORD")
        .context("PGAPP_ADMIN_DB_PASSWORD is not set — it's needed to connect as pgapp_admin and is never stored on disk")
}

/// Opens a pool connected as `pgapp_admin` against this instance's
/// database — what every workspace/app command operates through.
pub async fn connect_as_admin(instance: &InstanceFile) -> Result<PgPool> {
    let password = admin_db_password()?;
    let opts = PgConnectOptions::new()
        .host(&instance.host)
        .port(instance.port)
        .database(&instance.dbname)
        .username(&instance.admin_role)
        .password(&password);
    PgPoolOptions::new()
        .max_connections(max_connections())
        .connect_with(opts)
        .await
        .with_context(|| format!("failed to connect to '{}' as {}", instance.dbname, instance.admin_role))
}

/// A safe-to-splice-into-DDL identifier: same restriction the markup
/// lexer already applies to entity/page/field names, extended to
/// workspace slugs / schema / role names chosen interactively — none
/// of these can ever be bind parameters (Postgres has no placeholder
/// for identifiers), so this is the only thing standing between a
/// workspace name and a SQL injection.
pub fn valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    !s.is_empty() && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && s.len() <= 63
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn identifier_validation_matches_the_lexer_rule() {
        assert!(valid_identifier("acme"));
        assert!(valid_identifier("_private"));
        assert!(valid_identifier("acme_corp_1"));
        assert!(!valid_identifier(""));
        assert!(!valid_identifier("1acme"));
        assert!(!valid_identifier("acme-corp"));
        assert!(!valid_identifier("acme corp"));
        assert!(!valid_identifier("acme;drop table x"));
        assert!(!valid_identifier(&"a".repeat(64)));
    }

    #[test]
    fn save_and_load_roundtrip_in_an_isolated_home() {
        let dir = std::env::temp_dir().join(format!("pgapp-instance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("PGAPP_HOME", &dir);

        let instance = InstanceFile {
            dbname: "roundtrip_test".to_string(),
            host: "localhost".to_string(),
            port: 5432,
            admin_role: ADMIN_ROLE.to_string(),
            admin_password_hash: hash_password("whatever").unwrap(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        save(&instance).unwrap();
        let loaded = load("roundtrip_test").unwrap();
        assert_eq!(loaded.dbname, instance.dbname);
        assert_eq!(loaded.admin_role, ADMIN_ROLE);

        delete_file("roundtrip_test").unwrap();
        assert!(load("roundtrip_test").is_err());

        std::env::remove_var("PGAPP_HOME");
        std::fs::remove_dir_all(&dir).ok();
    }
}
