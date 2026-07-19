//! A pgapp *instance* is one target Postgres database plus a
//! dedicated `pgapp_admin` login role pgapp itself created and
//! operates as from then on — set up once via `pgapp instance init`.
//!
//! There is exactly one instance, globally, per machine (per
//! `PGAPP_HOME`) — not one per database. `pgapp instance init` refuses
//! if one is already set up (run `pgapp instance destroy` first), and
//! every other instance/workspace/app/secret/run command needs no
//! `<dbname>` argument at all: there's nothing to disambiguate.
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
//! lives at the fixed path `~/.pgapp/instance.json` (override the base
//! directory with `PGAPP_HOME`), `0600`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

pub const ADMIN_ROLE: &str = "pgapp_admin";

/// The App Builder's fixed, reserved workspace/app slugs (see
/// `main.rs`'s `provision_app_builder` and README's "App Builder"
/// section) — shared here rather than duplicated in `main.rs` (which
/// provisions it) and `server.rs` (which refuses to let it edit
/// itself), so the two can never drift apart.
pub const APP_BUILDER_WORKSPACE_SLUG: &str = "pgapp";
pub const APP_BUILDER_APP_SLUG: &str = "builder";

/// Pool size for connections that serve real HTTP traffic — the
/// `pgapp_admin` connection `pgapp run` reuses to serve an instance.
/// Default is in the same
/// ballpark as a typical APEX/ORDS pool for one moderately busy
/// workspace: comfortably above a handful of toy connections, without
/// assuming "bigger is always faster" (a Postgres backend is a full
/// process, not a lightweight thread, so a few dozen is already
/// generous for one server). Override with `PGAPP_MAX_CONNECTIONS` —
/// but raise Postgres's own capacity (`shared_buffers`, CPU) alongside
/// it, since a bigger pool alone stops helping once Postgres itself,
/// not the pool, is the bottleneck (confirmed by load-testing
/// examples/nexus-erp at 1,000 concurrent requests: pool=80 barely
/// beat pool=20, because both were waiting on Postgres, not the pool).
pub fn max_connections() -> u32 {
    std::env::var("PGAPP_MAX_CONNECTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(20)
}

/// Connection-pool configuration shared by every pool that serves live
/// HTTP traffic — as opposed to the small, fixed-size pools used for
/// one-off CLI/bootstrap connections elsewhere (scaffolding, database-
/// exists checks, control-plane lookups), which never see concurrent
/// request load and don't need any of this.
///
/// Cycling the fixed-size pool "smartly" across concurrent requests is
/// sqlx's job, not this function's: every query in server.rs is issued
/// against a borrowed `&PgPool` (`fetch_all`, `execute`, ...), which
/// acquires a connection just for that one query and returns it
/// immediately after — never held across a whole request — and sqlx's
/// internal wait queue for a busy pool is FIFO, so concurrent requests
/// are served in arrival order rather than one starving another.
///
/// No `min_connections` floor: keeping idle connections pre-warmed
/// only pays off in the first moments after a restart, and it's a
/// permanent cost the rest of the time — every idle connection is a
/// live Postgres backend process, counted against Postgres's own
/// `max_connections`, for as long as the server runs. That adds up
/// fast once several apps share one Postgres instance (pgapp's own
/// multi-app mode does exactly this), so connections are left to grow
/// from zero on demand instead, same as sqlx's own default.
///
/// `acquire_timeout` is kept explicit: a request that's waited 30s for
/// a connection fails loudly instead of queuing silently forever — a
/// visible error under sustained overload beats an invisible hang.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(max_connections())
        .acquire_timeout(std::time::Duration::from_secs(30))
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

pub fn home_dir() -> Result<PathBuf> {
    if let Ok(home) = std::env::var("PGAPP_HOME") {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::var("HOME").context("HOME is not set and PGAPP_HOME wasn't given")?;
    Ok(PathBuf::from(home).join(".pgapp"))
}

/// The single, fixed path every instance command reads/writes — there
/// is exactly one pgapp instance per machine (per `PGAPP_HOME`), so
/// unlike a per-database file there's nothing to key this by.
fn instance_path() -> Result<PathBuf> {
    Ok(home_dir()?.join("instance.json"))
}

/// Whether an instance is already set up — `instance_init` refuses to
/// overwrite one silently; the operator has to `pgapp instance destroy`
/// first.
pub fn exists() -> Result<bool> {
    Ok(Path::new(&instance_path()?).exists())
}

/// Writes the instance file, creating `~/.pgapp` (`0700`) if needed and
/// the file itself `0600` — this holds a password *hash*, never a
/// usable secret, but there's no reason to leave it world readable.
pub fn save(instance: &InstanceFile) -> Result<()> {
    let dir = home_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create '{}'", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    }

    let path = instance_path()?;
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

pub fn load() -> Result<InstanceFile> {
    let path = instance_path()?;
    if !Path::new(&path).exists() {
        bail!("no pgapp instance is set up (expected '{}') — run `pgapp instance init` first", path.display());
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("failed to read '{}'", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse '{}'", path.display()))
}

/// Deletes the instance file — the local half of `pgapp instance
/// destroy`; the caller is responsible for the Postgres-side teardown
/// (dropping the role/schemas) before or after this.
pub fn delete_file() -> Result<()> {
    let path = instance_path()?;
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
    pool_options()
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
        assert!(!exists().unwrap());
        save(&instance).unwrap();
        assert!(exists().unwrap());
        let loaded = load().unwrap();
        assert_eq!(loaded.dbname, instance.dbname);
        assert_eq!(loaded.admin_role, ADMIN_ROLE);

        delete_file().unwrap();
        assert!(!exists().unwrap());
        assert!(load().is_err());

        std::env::remove_var("PGAPP_HOME");
        std::fs::remove_dir_all(&dir).ok();
    }
}
