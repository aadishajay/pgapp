//! Authentication and authorization.
//!
//! - **AuthN**: username/password against `pgapp_meta.users` (argon2
//!   hashes, never plaintext), server-side sessions in
//!   `pgapp_meta.sessions` with only a random token in an HttpOnly
//!   cookie — a session is revoked by deleting its row. The first
//!   visit to /login on an app with no users offers a one-time "create
//!   the admin account" form; after that, only admins can add users
//!   (via the built-in /users page). Users are deliberately *not*
//!   declarable in markup: passwords don't belong in a source file.
//! - **AuthZ**: a user has one `role` (free-form string). A page's
//!   `requires: <role>` markup restricts it to that role; 'admin'
//!   passes every check. Pages without `requires:` need any signed-in
//!   user. Apps without an `auth { }` block skip all of this and stay
//!   public.
//!
//! Everything hangs off [`require_login`], an axum middleware that
//! resolves the session cookie into an [`AuthCtx`] request extension
//! and redirects unauthenticated page requests to /login.

use std::collections::HashMap;
use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::extract::{Form, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use sqlx::PgPool;
use uuid::Uuid;

use super::query_engine::resolve_regions;
use super::{err_response, visible_nav, AppData, AppError, AppState};
use crate::html::url_encode;
use crate::meta::Chrome;
use crate::render;

pub const SESSION_COOKIE: &str = "pgapp_session";
const SESSION_SECONDS: i64 = 7 * 24 * 3600;
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i32,
    pub username: String,
    pub role: String,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// The signed-in user for this request, if any — inserted by
/// [`require_login`] on every request (as `AuthCtx(None)` when the app
/// has auth disabled, so handlers can always extract it).
#[derive(Debug, Clone, Default)]
pub struct AuthCtx(pub Option<CurrentUser>);

impl AuthCtx {
    /// (username, is_admin) for the nav bar — what render.rs needs,
    /// without depending on this module's types.
    pub fn display(&self) -> Option<(&str, bool)> {
        self.0.as_ref().map(|u| (u.username.as_str(), u.is_admin()))
    }
}

// ---- password hashing ----

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))?
        .to_string())
}

fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

// ---- sessions ----

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(raw) = value.to_str() else { continue };
        for pair in raw.split(';') {
            if let Some((name, token)) = pair.trim().split_once('=') {
                if name.trim() == SESSION_COOKIE && !token.is_empty() {
                    return Some(token.trim().to_string());
                }
            }
        }
    }
    None
}

/// Creates a session row and returns the Set-Cookie header value. Two
/// v4 UUIDs give the token ~244 bits of randomness.
async fn create_session(pool: &PgPool, app_id: i32, user_id: i32) -> anyhow::Result<String> {
    // Opportunistic cleanup: expired sessions are dead weight either way.
    sqlx::query("delete from pgapp_meta.sessions where expires_at < now()")
        .execute(pool)
        .await?;

    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "insert into pgapp_meta.sessions (token, app_id, user_id, expires_at)
         values ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(&token)
    .bind(app_id)
    .bind(user_id)
    .bind(SESSION_SECONDS as f64)
    .execute(pool)
    .await?;

    Ok(format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_SECONDS}"
    ))
}

async fn load_session(pool: &PgPool, app_id: i32, token: &str) -> anyhow::Result<Option<CurrentUser>> {
    let row: Option<(i32, String, String)> = sqlx::query_as(
        "select u.id, u.username, u.role
           from pgapp_meta.sessions s
           join pgapp_meta.users u on u.id = s.user_id
          where s.token = $1 and s.app_id = $2 and s.expires_at > now()",
    )
    .bind(token)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, username, role)| CurrentUser { id, username, role }))
}

async fn user_count(pool: &PgPool, app_id: i32) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("select count(*) from pgapp_meta.users where app_id = $1")
        .bind(app_id)
        .fetch_one(pool)
        .await?)
}

// ---- middleware ----

/// Paths that must stay reachable without a session, or the login page
/// couldn't render (its stylesheet!) and nobody could ever sign in.
fn is_public(path: &str) -> bool {
    matches!(path, "/login" | "/setup" | "/theme.css" | "/runtime.js" | "/chart-lib.js")
        || path.starts_with("/assets/")
}

pub async fn require_login(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let mut ctx = AuthCtx(None);
    let data = state.data();

    if data.app.auth_enabled {
        if let Some(token) = cookie_token(req.headers()) {
            if let Ok(user) = load_session(&state.pool, data.app.id, &token).await {
                ctx = AuthCtx(user);
            }
        }
        if ctx.0.is_none() && !is_public(req.uri().path()) {
            return Redirect::to("/login").into_response();
        }
    }

    req.extensions_mut().insert(ctx);
    next.run(req).await
}

/// The per-page role gate, called by every page-serving/writing
/// handler. With auth disabled everything is public; with it enabled,
/// the middleware already guaranteed a signed-in user, so only the
/// role remains to check ('admin' passes everything).
pub fn authorize(data: &AppData, required_role: Option<&str>, auth: &AuthCtx) -> Result<(), AppError> {
    if !data.app.auth_enabled {
        return Ok(());
    }
    let user = auth.0.as_ref().ok_or((StatusCode::UNAUTHORIZED, "sign in required".to_string()))?;
    match required_role {
        None => Ok(()),
        Some(role) if user.role == role || user.is_admin() => Ok(()),
        Some(role) => Err((
            StatusCode::FORBIDDEN,
            format!("this page requires the '{role}' role (you are '{}')", user.role),
        )),
    }
}

fn require_admin(auth: &AuthCtx) -> Result<&CurrentUser, AppError> {
    match &auth.0 {
        Some(user) if user.is_admin() => Ok(user),
        Some(_) => Err((StatusCode::FORBIDDEN, "this page requires the 'admin' role".to_string())),
        None => Err((StatusCode::UNAUTHORIZED, "sign in required".to_string())),
    }
}

// ---- handlers ----

pub async fn login_form(State(state): State<Arc<AppState>>) -> Result<Html<String>, AppError> {
    let data = state.data();
    if !data.app.auth_enabled {
        return Err((StatusCode::NOT_FOUND, "this app does not use authentication".to_string()));
    }
    let setup = user_count(&state.pool, data.app.id).await.map_err(err_response)? == 0;
    Ok(Html(render::login_page(&data.app.name, None, setup)))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let data = state.data();
    if !data.app.auth_enabled {
        return Err((StatusCode::NOT_FOUND, "this app does not use authentication".to_string()));
    }
    let username = values.get("username").map(|s| s.trim()).unwrap_or_default();
    let password = values.get("password").map(|s| s.as_str()).unwrap_or_default();

    let row: Option<(i32, String)> = sqlx::query_as(
        "select id, password_hash from pgapp_meta.users where app_id = $1 and username = $2",
    )
    .bind(data.app.id)
    .bind(username)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| err_response(e.into()))?;

    // Same failure path whether the user is unknown or the password is
    // wrong — don't leak which usernames exist.
    let Some((user_id, stored_hash)) = row else {
        return Ok(Html(render::login_page(&data.app.name, Some("Invalid username or password."), false)).into_response());
    };
    if !verify_password(password, &stored_hash) {
        return Ok(Html(render::login_page(&data.app.name, Some("Invalid username or password."), false)).into_response());
    }

    let cookie = create_session(&state.pool, data.app.id, user_id)
        .await
        .map_err(err_response)?;
    Ok((StatusCode::SEE_OTHER, [(header::SET_COOKIE, cookie), (header::LOCATION, "/".to_string())]).into_response())
}

/// One-time first-run bootstrap: creates the admin account, but only
/// while the app has zero users — the moment one exists this endpoint
/// refuses, so it can't be used to sneak in a second admin.
pub async fn setup(
    State(state): State<Arc<AppState>>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let data = state.data();
    if !data.app.auth_enabled {
        return Err((StatusCode::NOT_FOUND, "this app does not use authentication".to_string()));
    }
    if user_count(&state.pool, data.app.id).await.map_err(err_response)? > 0 {
        return Err((StatusCode::FORBIDDEN, "setup already completed".to_string()));
    }

    let username = values.get("username").map(|s| s.trim()).unwrap_or_default();
    let password = values.get("password").map(|s| s.as_str()).unwrap_or_default();
    if username.is_empty() || password.len() < MIN_PASSWORD_LEN {
        return Ok(Html(render::login_page(
            &data.app.name,
            Some("Username is required and the password needs at least 8 characters."),
            true,
        ))
        .into_response());
    }

    let hash = hash_password(password).map_err(err_response)?;
    let user_id: i32 = sqlx::query_scalar(
        "insert into pgapp_meta.users (app_id, username, password_hash, role)
         values ($1, $2, $3, 'admin') returning id",
    )
    .bind(data.app.id)
    .bind(username)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| err_response(e.into()))?;

    let cookie = create_session(&state.pool, data.app.id, user_id)
        .await
        .map_err(err_response)?;
    Ok((StatusCode::SEE_OTHER, [(header::SET_COOKIE, cookie), (header::LOCATION, "/".to_string())]).into_response())
}

pub async fn logout(State(state): State<Arc<AppState>>, req: Request) -> Result<Response, AppError> {
    if let Some(token) = cookie_token(req.headers()) {
        sqlx::query("delete from pgapp_meta.sessions where token = $1")
            .bind(&token)
            .execute(&state.pool)
            .await
            .map_err(|e| err_response(e.into()))?;
    }
    let clear = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    Ok((StatusCode::SEE_OTHER, [(header::SET_COOKIE, clear), (header::LOCATION, "/login".to_string())]).into_response())
}

// ---- the built-in /users admin page ----

pub async fn users_page(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth): axum::Extension<AuthCtx>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let current = require_admin(&auth)?;
    let data = state.data();

    let users: Vec<(i32, String, String)> = sqlx::query_as(
        "select id, username, role from pgapp_meta.users where app_id = $1 order by username",
    )
    .bind(data.app.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| err_response(e.into()))?;

    let ctx = HashMap::new();
    let regions = resolve_regions(&state.pool, &data.app, None, &ctx)
        .await
        .map_err(err_response)?;

    let nav = visible_nav(&data.app, &data.app.nav, &data, &auth);
    Ok(Html(render::users_page(
        &data.app.name,
        &users,
        current.id,
        query.get("error").map(|s| s.as_str()),
        Chrome { nav: &nav, ..data.app.chrome(&regions) },
        &data.icons,
        &data.chart_lib,
        auth.display(),
    )))
}

pub async fn users_create(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth): axum::Extension<AuthCtx>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    require_admin(&auth)?;

    let username = values.get("username").map(|s| s.trim()).unwrap_or_default();
    let password = values.get("password").map(|s| s.as_str()).unwrap_or_default();
    let role = values.get("role").map(|s| s.trim()).unwrap_or_default();
    let role = if role.is_empty() { "user" } else { role };

    if username.is_empty() || password.len() < MIN_PASSWORD_LEN {
        let msg = "Username is required and the password needs at least 8 characters.";
        return Ok(Redirect::to(&format!("/users?error={}", url_encode(msg))).into_response());
    }

    let hash = hash_password(password).map_err(err_response)?;
    let result = sqlx::query(
        "insert into pgapp_meta.users (app_id, username, password_hash, role) values ($1, $2, $3, $4)",
    )
    .bind(state.data().app.id)
    .bind(username)
    .bind(&hash)
    .bind(role)
    .execute(&state.pool)
    .await;

    match result {
        Ok(_) => Ok(Redirect::to("/users").into_response()),
        Err(_) => {
            let msg = format!("Could not create '{username}' — that username is already taken.");
            Ok(Redirect::to(&format!("/users?error={}", url_encode(&msg))).into_response())
        }
    }
}

pub async fn users_delete(
    State(state): State<Arc<AppState>>,
    axum::Extension(auth): axum::Extension<AuthCtx>,
    Path(user_id): Path<i32>,
) -> Result<Response, AppError> {
    let current = require_admin(&auth)?;
    if current.id == user_id {
        let msg = "You cannot delete the account you are signed in with.";
        return Ok(Redirect::to(&format!("/users?error={}", url_encode(msg))).into_response());
    }

    sqlx::query("delete from pgapp_meta.users where id = $1 and app_id = $2")
        .bind(user_id)
        .bind(state.data().app.id)
        .execute(&state.pool)
        .await
        .map_err(|e| err_response(e.into()))?;
    Ok(Redirect::to("/users").into_response())
}
