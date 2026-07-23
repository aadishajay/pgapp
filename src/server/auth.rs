//! Authentication and authorization.
//!
//! - **AuthN**: username/password against `pgapp_meta.users` (argon2
//!   hashes, never plaintext), server-side sessions in
//!   `pgapp_meta.sessions` with only a random token in an HttpOnly
//!   cookie — a session is revoked by deleting its row. The cookie is
//!   scoped to `Path=/{workspace}/{app}` (see `create_session`), so a
//!   browser only ever sends one app's token to that app's own routes,
//!   even when several apps share the same process/pool. The first
//!   visit to `/{workspace}/{app}/login` on an app with no users offers
//!   a one-time "create the admin account" form; after that, only
//!   admins can add users (via the built-in `/{workspace}/{app}/users`
//!   page). Users are deliberately *not* declarable in markup:
//!   passwords don't belong in a source file.
//! - **AuthZ**: a user holds any number of free-form `roles`. A page's
//!   or component's `requires: <role_or_scheme>` markup restricts it to
//!   users holding that role (or any role in that named `auth_scheme`,
//!   if the name matches one — see `model::AuthScheme`); 'admin' passes
//!   every check regardless. Pages/components without `requires:` need
//!   any signed-in user. Apps without an `auth { }` block skip all of
//!   this and stay public.
//!
//! Everything hangs off [`require_login`], an axum middleware that
//! resolves the app's `{workspace}/{app}` key from the raw request path
//! (it runs before any route's own `Path` extraction), looks up that
//! app's session cookie into an [`AuthCtx`] request extension, and
//! redirects unauthenticated page requests to that app's own `/login`.

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

/// A stable per-browser identity, independent of login — minted the
/// first time any request for an app arrives without one, kept for a
/// year, scoped `Path=/{workspace}/{app}` like the session cookie. Exists purely
/// so collections (`pgapp_meta.collections`, see db/schema.sql) have
/// something to scope "only the caller can see this" to that works
/// the same whether or not the app uses `auth { }` — a signed-in
/// user's session cookie would do for an auth-enabled app, but public
/// apps have no session concept to reuse, and collections shouldn't
/// have a different guarantee depending on that.
pub const CALLER_COOKIE: &str = "pgapp_caller";
const CALLER_COOKIE_SECONDS: i64 = 365 * 24 * 3600;

/// The current request's collection-scoping identity — inserted by
/// [`require_login`] on every request, same as [`AuthCtx`].
#[derive(Debug, Clone)]
pub struct CallerKey(pub String);

#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i32,
    pub username: String,
    pub roles: Vec<String>,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.roles.iter().any(|r| r == "admin")
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.is_admin() || self.roles.iter().any(|r| r == role)
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

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(raw) = value.to_str() else { continue };
        for pair in raw.split(';') {
            if let Some((n, v)) = pair.trim().split_once('=') {
                if n.trim() == name && !v.is_empty() {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// pgapp itself never terminates TLS (see `main::serve_registered_apps`'s
/// plain `TcpListener`) — a production deployment puts a reverse proxy
/// in front that does, and forwards `X-Forwarded-Proto: https` the same
/// way every mainstream proxy (Caddy, nginx, Traefik, cloud load
/// balancers) does by default. Trusting that header is only sound
/// because the proxy sits between the browser and this process and
/// overwrites whatever a client sent — never expose pgapp directly to
/// the internet without one in front, or this check is trivially spoofed.
fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Creates a session row and returns the Set-Cookie header value. Two
/// v4 UUIDs give the token ~244 bits of randomness. `Path=/{app_key}`
/// (`app_key` is `"{workspace}/{app_slug}"`) keeps the cookie from ever
/// being sent to a *different* app sharing this process — `load_session`'s
/// own `app_id` filter would reject it anyway, but scoping the cookie
/// itself means the browser doesn't even offer it cross-app. `secure`
/// (see `is_https`) appends `Secure` so the browser never sends the
/// token back over a plain-HTTP connection.
async fn create_session(pool: &PgPool, app_id: i32, user_id: i32, app_key: &str, secure: bool) -> anyhow::Result<String> {
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

    let secure_attr = if secure { "; Secure" } else { "" };
    Ok(format!(
        "{SESSION_COOKIE}={token}; Path=/{app_key}; HttpOnly; SameSite=Lax; Max-Age={SESSION_SECONDS}{secure_attr}"
    ))
}

async fn load_session(pool: &PgPool, app_id: i32, token: &str) -> anyhow::Result<Option<CurrentUser>> {
    let row: Option<(i32, String, Vec<String>)> = sqlx::query_as(
        "select u.id, u.username, u.roles
           from pgapp_meta.sessions s
           join pgapp_meta.users u on u.id = s.user_id
          where s.token = $1 and s.app_id = $2 and s.expires_at > now()",
    )
    .bind(token)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, username, roles)| CurrentUser { id, username, roles }))
}

async fn user_count(pool: &PgPool, app_id: i32) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("select count(*) from pgapp_meta.users where app_id = $1")
        .bind(app_id)
        .fetch_one(pool)
        .await?)
}

// ---- middleware ----

/// Sub-paths (everything after `/{workspace}/{app}`) that must stay
/// reachable without a session, or the login page couldn't render (its
/// stylesheet!) and nobody could ever sign in. The empty string is
/// `/{workspace}/{app}` itself — `index` doesn't check auth; it just
/// redirects to the app's first page, which `show` re-checks.
fn is_public(rest: &str) -> bool {
    matches!(rest, "" | "/login" | "/setup" | "/theme.css" | "/runtime.js" | "/chart-lib.js")
        || rest.starts_with("/assets/")
}

pub async fn require_login(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if path == "/" {
        // The workspace landing page: public regardless of any app's
        // own auth setting, and not scoped to any one app.
        req.extensions_mut().insert(AuthCtx(None));
        return next.run(req).await;
    }

    // The app's two leading path segments — `/{workspace}/{app}/...` —
    // resolved before any route's own `Path` extraction runs.
    let trimmed = path.trim_start_matches('/');
    let mut segments = trimmed.splitn(3, '/');
    let (Some(workspace), Some(app)) = (segments.next(), segments.next()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let rest = match segments.next() {
        Some(r) => format!("/{r}"),
        None => String::new(),
    };
    let app_key = format!("{workspace}/{app}");

    let Some(entry) = state.apps.read().unwrap().get(app_key.as_str()).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let data = entry.data();

    let mut ctx = AuthCtx(None);
    if data.app.auth_enabled {
        if let Some(token) = cookie_value(req.headers(), SESSION_COOKIE) {
            if let Ok(user) = load_session(&state.pool, data.app.id, &token).await {
                ctx = AuthCtx(user);
            }
        }
        if ctx.0.is_none() && !is_public(&rest) {
            return Redirect::to(&format!("/{app_key}/login")).into_response();
        }
    }

    let minted_cookie = match cookie_value(req.headers(), CALLER_COOKIE) {
        Some(existing) => {
            req.extensions_mut().insert(CallerKey(existing));
            None
        }
        None => {
            let fresh = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let secure_attr = if is_https(req.headers()) { "; Secure" } else { "" };
            let cookie = format!(
                "{CALLER_COOKIE}={fresh}; Path=/{app_key}; HttpOnly; SameSite=Lax; Max-Age={CALLER_COOKIE_SECONDS}{secure_attr}"
            );
            req.extensions_mut().insert(CallerKey(fresh));
            Some(cookie)
        }
    };

    req.extensions_mut().insert(ctx);
    let mut response = next.run(req).await;
    if let Some(cookie) = minted_cookie {
        if let Ok(value) = header::HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

/// The per-page/per-component role gate, called by every page-serving/
/// writing handler (and, for the latter, by `server::render_component`
/// and every mutating route). With auth disabled everything is public;
/// with it enabled, the middleware already guaranteed a signed-in user.
///
/// `required` is either a literal role or the name of an app-declared
/// `auth_scheme` (see `model::AuthScheme`) — resolved through
/// `data.app.schemes` first; a name that matches no scheme falls back
/// to being checked as a literal role directly, so an app with no
/// schemes at all behaves exactly as if this resolution step didn't
/// exist. 'admin' passes regardless of which role(s) are required.
pub fn authorize(data: &AppData, required: Option<&str>, auth: &AuthCtx) -> Result<(), AppError> {
    if !data.app.auth_enabled {
        return Ok(());
    }
    let user = auth.0.as_ref().ok_or((StatusCode::UNAUTHORIZED, "sign in required".to_string()))?;
    let Some(name) = required else { return Ok(()) };
    if user.is_admin() {
        return Ok(());
    }
    let owned;
    let allowed_roles: &[String] = match data.app.schemes.get(name) {
        Some(roles) => roles,
        None => {
            owned = vec![name.to_string()];
            &owned
        }
    };
    if allowed_roles.iter().any(|r| user.roles.contains(r)) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!("this requires the '{name}' role (you have: {})", user.roles.join(", ")),
        ))
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

pub async fn login_form(
    State(state): State<Arc<AppState>>,
    Path((workspace, app)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
    if !data.app.auth_enabled {
        return Err((StatusCode::NOT_FOUND, "this app does not use authentication".to_string()));
    }
    let setup = user_count(&state.pool, data.app.id).await.map_err(err_response)? == 0;
    Ok(Html(render::login_page(&app, &data.app.name, None, setup)))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Path((workspace, app)): Path<(String, String)>,
    headers: HeaderMap,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
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
        return Ok(Html(render::login_page(&app, &data.app.name, Some("Invalid username or password."), false)).into_response());
    };
    if !verify_password(password, &stored_hash) {
        return Ok(Html(render::login_page(&app, &data.app.name, Some("Invalid username or password."), false)).into_response());
    }

    let cookie = create_session(&state.pool, data.app.id, user_id, &app, is_https(&headers))
        .await
        .map_err(err_response)?;
    Ok((StatusCode::SEE_OTHER, [(header::SET_COOKIE, cookie), (header::LOCATION, format!("/{app}"))]).into_response())
}

/// One-time first-run bootstrap: creates the admin account, but only
/// while the app has zero users — the moment one exists this endpoint
/// refuses, so it can't be used to sneak in a second admin.
pub async fn setup(
    State(state): State<Arc<AppState>>,
    Path((workspace, app)): Path<(String, String)>,
    headers: HeaderMap,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();
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
            &app,
            &data.app.name,
            Some("Username is required and the password needs at least 8 characters."),
            true,
        ))
        .into_response());
    }

    let hash = hash_password(password).map_err(err_response)?;
    let user_id: i32 = sqlx::query_scalar(
        "insert into pgapp_meta.users (app_id, username, password_hash, roles)
         values ($1, $2, $3, array['admin']) returning id",
    )
    .bind(data.app.id)
    .bind(username)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| err_response(e.into()))?;

    let cookie = create_session(&state.pool, data.app.id, user_id, &app, is_https(&headers))
        .await
        .map_err(err_response)?;
    Ok((StatusCode::SEE_OTHER, [(header::SET_COOKIE, cookie), (header::LOCATION, format!("/{app}"))]).into_response())
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    Path((workspace, app)): Path<(String, String)>,
    req: Request,
) -> Result<Response, AppError> {
    let app = format!("{workspace}/{app}");
    state.app_or_404(&app)?;
    if let Some(token) = cookie_value(req.headers(), SESSION_COOKIE) {
        sqlx::query("delete from pgapp_meta.sessions where token = $1")
            .bind(&token)
            .execute(&state.pool)
            .await
            .map_err(|e| err_response(e.into()))?;
    }
    let clear = format!("{SESSION_COOKIE}=; Path=/{app}; HttpOnly; SameSite=Lax; Max-Age=0");
    Ok((StatusCode::SEE_OTHER, [(header::SET_COOKIE, clear), (header::LOCATION, format!("/{app}/login"))]).into_response())
}

// ---- the built-in /:workspace/:app/users admin page ----

pub async fn users_page(
    State(state): State<Arc<AppState>>,
    Path((workspace, app)): Path<(String, String)>,
    axum::Extension(auth): axum::Extension<AuthCtx>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Html<String>, AppError> {
    let current = require_admin(&auth)?;
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    let data = entry.data();

    let users: Vec<(i32, String, Vec<String>)> = sqlx::query_as(
        "select id, username, roles from pgapp_meta.users where app_id = $1 order by username",
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
        &app,
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
    Path((workspace, app)): Path<(String, String)>,
    axum::Extension(auth): axum::Extension<AuthCtx>,
    Form(values): Form<HashMap<String, String>>,
) -> Result<Response, AppError> {
    require_admin(&auth)?;
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;

    let username = values.get("username").map(|s| s.trim()).unwrap_or_default();
    let password = values.get("password").map(|s| s.as_str()).unwrap_or_default();
    let roles: Vec<String> = values
        .get("roles")
        .map(|s| s.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect())
        .unwrap_or_default();
    let roles = if roles.is_empty() { vec!["user".to_string()] } else { roles };

    if username.is_empty() || password.len() < MIN_PASSWORD_LEN {
        let msg = "Username is required and the password needs at least 8 characters.";
        return Ok(Redirect::to(&format!("/{app}/users?error={}", url_encode(msg))).into_response());
    }

    let hash = hash_password(password).map_err(err_response)?;
    let result = sqlx::query(
        "insert into pgapp_meta.users (app_id, username, password_hash, roles) values ($1, $2, $3, $4)",
    )
    .bind(entry.data().app.id)
    .bind(username)
    .bind(&hash)
    .bind(&roles)
    .execute(&state.pool)
    .await;

    match result {
        Ok(_) => Ok(Redirect::to(&format!("/{app}/users")).into_response()),
        Err(_) => {
            let msg = format!("Could not create '{username}' — that username is already taken.");
            Ok(Redirect::to(&format!("/{app}/users?error={}", url_encode(&msg))).into_response())
        }
    }
}

pub async fn users_delete(
    State(state): State<Arc<AppState>>,
    Path((workspace, app, user_id)): Path<(String, String, i32)>,
    axum::Extension(auth): axum::Extension<AuthCtx>,
) -> Result<Response, AppError> {
    let current = require_admin(&auth)?;
    let app = format!("{workspace}/{app}");
    let entry = state.app_or_404(&app)?;
    if current.id == user_id {
        let msg = "You cannot delete the account you are signed in with.";
        return Ok(Redirect::to(&format!("/{app}/users?error={}", url_encode(msg))).into_response());
    }

    sqlx::query("delete from pgapp_meta.users where id = $1 and app_id = $2")
        .bind(user_id)
        .bind(entry.data().app.id)
        .execute(&state.pool)
        .await
        .map_err(|e| err_response(e.into()))?;
    Ok(Redirect::to(&format!("/{app}/users")).into_response())
}
