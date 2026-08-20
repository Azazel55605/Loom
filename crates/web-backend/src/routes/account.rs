//! Self-service account management: the routes a user points at themselves.
//!
//! ## Why these need no permission
//!
//! Every handler here reads the subject out of the caller's own access token
//! and acts on that row. **There is no user-id path parameter anywhere in this
//! module**, which is the structural reason no permission check is needed: the
//! target is not an input, so there is no value a caller could supply to reach
//! someone else's account. Changing your own display name is not an
//! administrative act, and gating it behind `users.manage` would mean an
//! instance where ordinary users cannot set their own password.
//!
//! Contrast `routes/users.rs`, which takes an id and therefore does require a
//! grant. If a route here ever grows an id parameter, it stops belonging in
//! this module.

use std::path::{Path, PathBuf};

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;
use uuid::Uuid;

use crate::auth::extract::AuthenticatedUser;
use crate::auth::password::{hash_password, verify_password, MIN_PASSWORD_LENGTH};
use crate::config::AVATARS_DIRNAME;
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

/// Largest avatar accepted, in bytes.
///
/// A limit on the *decoded-from* bytes, checked before decoding. Two megabytes
/// is far more than a profile picture needs and small enough that a handful of
/// concurrent uploads cannot be used to fill the data volume.
pub const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;

/// Extra body allowance on the upload route, above [`MAX_AVATAR_BYTES`].
///
/// The transport limit has to sit above the file limit or multipart framing —
/// boundaries, headers, the field name — would count against the file's budget
/// and reject a file that is actually within it. The slack is generous so that
/// a slightly-oversized upload reaches the handler and gets a JSON 413
/// explaining the limit, rather than the bare 413 the body-limit layer
/// produces. Anything past *this* is refused unread, which is the point: a
/// gigabyte upload should not be buffered in order to be told no.
pub const AVATAR_BODY_SLACK_BYTES: usize = 1024 * 1024;

/// Ceiling on memory an avatar may decode into.
///
/// Distinct from [`MAX_AVATAR_BYTES`], and not redundant with it. Compressed
/// size says nothing about decoded size: a valid, tiny PNG can declare
/// enormous dimensions and expand into gigabytes — the "decompression bomb"
/// that makes naive image handling a denial-of-service vector. The byte limit
/// bounds what we read; this bounds what decoding it may cost.
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

/// The caller's own profile.
///
/// `groups` is present for context — a user can see what they belong to — and
/// is read-only here. Membership is an administrative decision made through
/// `PATCH /users/{id}`, and offering it on a self-service route would be
/// offering privilege escalation with a friendly label.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountResponse {
    id: String,
    username: String,
    display_name: Option<String>,
    /// Relative URL, e.g. `/avatars/{uuid}.png`, or null when none is set.
    ///
    /// Relative on purpose: the backend does not know the origin it is reached
    /// through — direct, through the frontend's `/api` proxy, or through a
    /// reverse proxy on another host — so any absolute URL it invented would be
    /// wrong for some caller. Resolve it against the same base as the API.
    avatar_url: Option<String>,
    created_at: String,
    groups: Vec<AccountGroup>,
}

/// A group the caller belongs to, named rather than just identified, since the
/// point of including it is for a person to read.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGroup {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccountRequest {
    /// Absent leaves it alone.
    username: Option<String>,
    /// Absent leaves it alone; present-and-null clears it, as does an
    /// all-whitespace string — a display name of `"   "` is not a name, and
    /// storing one would render as a blank where a username should be.
    #[serde(default, deserialize_with = "crate::routes::present_option")]
    display_name: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

/// `POST /account/avatar` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarResponse {
    avatar_url: String,
}

#[derive(sqlx::FromRow)]
struct AccountRow {
    id: String,
    username: String,
    display_name: Option<String>,
    avatar_path: Option<String>,
    created_at: String,
}

/// `GET /account`
pub async fn get_account(caller: AuthenticatedUser, State(state): State<AppState>) -> Response {
    let mut conn = match state.pool.acquire().await {
        Ok(conn) => conn,
        Err(error) => return internal_error("acquiring a connection", error),
    };

    match load_account(&mut conn, caller.id()).await {
        Ok(Some(account)) => Json(account).into_response(),
        // The token verified, so the account existed when it was issued. Being
        // gone now means it was deleted mid-session: a 404 is the honest answer
        // and tells the client to stop using the session.
        Ok(None) => account_gone(),
        Err(error) => internal_error("loading the account", error),
    }
}

/// `PATCH /account`
///
/// ## Renaming yourself and the token you are holding
///
/// The access token embeds `username` at issuance, so a token minted before
/// this call keeps reporting the old name until it expires. Nothing here
/// invalidates it, and nothing needs to: access tokens live 15 minutes, and the
/// next refresh mints one carrying the new name. This is the same staleness
/// window that already applies to permission changes — see
/// `docs/adr/0008-auth-model.md` — so it is a property of the design rather
/// than a gap in this handler.
///
/// The claim is used for display, never for identification: every handler keys
/// off `sub`, the user id, which does not change. A stale `username` claim can
/// therefore show an out-of-date name for a few minutes; it cannot address the
/// wrong account.
pub async fn update_account(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<UpdateAccountRequest>,
) -> Response {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the update-account transaction", error),
    };

    if let Some(username) = &request.username {
        let username = username.trim();

        if username.is_empty() {
            return ErrorBody::message(
                StatusCode::BAD_REQUEST,
                "username must not be empty".to_owned(),
            );
        }

        // Excluding self, or renaming an account to the name it already has
        // would conflict with itself. Checked explicitly so the caller gets a
        // 409 naming the problem rather than a 500 from the UNIQUE index, which
        // still backstops a race — same reasoning as `create_user`.
        match sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM users WHERE username = ? AND id != ?",
        )
        .bind(username)
        .bind(caller.id())
        .fetch_one(&mut *tx)
        .await
        {
            Ok((0,)) => {}
            Ok(_) => {
                return ErrorBody::message(
                    StatusCode::CONFLICT,
                    format!("a user named {username} already exists"),
                );
            }
            Err(error) => return internal_error("checking username uniqueness", error),
        }

        if let Err(error) = sqlx::query("UPDATE users SET username = ? WHERE id = ?")
            .bind(username)
            .bind(caller.id())
            .execute(&mut *tx)
            .await
        {
            return internal_error("updating the username", error);
        }
    }

    if let Some(display_name) = &request.display_name {
        let value = display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());

        if let Err(error) = sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
            .bind(value)
            .bind(caller.id())
            .execute(&mut *tx)
            .await
        {
            return internal_error("updating the display name", error);
        }
    }

    let account = match load_account(&mut tx, caller.id()).await {
        Ok(Some(account)) => account,
        Ok(None) => return account_gone(),
        Err(error) => return internal_error("reloading the account", error),
    };

    if let Err(error) = tx.commit().await {
        return internal_error("committing the account update", error);
    }

    Json(account).into_response()
}

/// `POST /account/password`
///
/// ## Why existing sessions survive a password change
///
/// Nothing here revokes tokens, so a session established before the change
/// keeps working. That is an accepted trade-off rather than an oversight, and
/// it is worth being precise about its size: the *access* token's 15-minute
/// life is not the bound, because the refresh token that renews it lives seven
/// days. Someone holding a stolen refresh token keeps their access across a
/// password change for up to that long.
///
/// The reason to accept it for now is that the common case — a user rotating
/// their own password on their own machine — gains nothing from being signed
/// out, while the mechanism that would help is a broader one: revoking a user's
/// refresh tokens on demand, which is also what "sign out my other devices"
/// needs and what an administrator wants when an account is compromised.
/// Bolting a single `DELETE FROM refresh_tokens` onto this handler would cover
/// one path to that need and leave the others, so it belongs with that feature.
/// **Revisit when session revocation lands.**
pub async fn change_password(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<ChangePasswordRequest>,
) -> Response {
    let stored = sqlx::query_as::<_, (String,)>("SELECT password_hash FROM users WHERE id = ?")
        .bind(caller.id())
        .fetch_optional(&state.pool)
        .await;

    let (password_hash,) = match stored {
        Ok(Some(row)) => row,
        Ok(None) => return account_gone(),
        Err(error) => return internal_error("loading the stored password", error),
    };

    // A distinct 401 from the login route's. Login deliberately gives one
    // identical answer for every kind of failure, because distinguishing them
    // there tells an anonymous caller which usernames exist. Here the caller is
    // already authenticated as this exact account, so there is nothing left to
    // disclose — and "current password incorrect" is the only message that
    // tells them which of the two fields to fix.
    if !verify_password(&request.current_password, &password_hash) {
        return ErrorBody::message(
            StatusCode::UNAUTHORIZED,
            "current password is incorrect".to_owned(),
        );
    }

    // The same floor as setup and user creation, from the same constant, so the
    // rule cannot drift between the three ways a password gets set.
    if request.new_password.len() < MIN_PASSWORD_LENGTH {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("password must be at least {MIN_PASSWORD_LENGTH} characters"),
        );
    }

    let new_hash = match hash_password(&request.new_password) {
        Ok(hash) => hash,
        Err(error) => return internal_error("hashing the new password", error),
    };

    if let Err(error) = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(&new_hash)
        .bind(caller.id())
        .execute(&state.pool)
        .await
    {
        return internal_error("storing the new password", error);
    }

    tracing::info!(user_id = %caller.id(), "password changed");

    StatusCode::NO_CONTENT.into_response()
}

/// `POST /account/avatar` — multipart upload of a single image file.
pub async fn upload_avatar(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    multipart: Multipart,
) -> Response {
    let bytes = match read_upload(multipart).await {
        Ok(bytes) => bytes,
        Err(response) => return *response,
    };

    if bytes.len() > MAX_AVATAR_BYTES {
        return ErrorBody::message(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "avatar must be at most {} MB",
                MAX_AVATAR_BYTES / (1024 * 1024)
            ),
        );
    }

    // Decoding is CPU-bound and can take tens of milliseconds on a large image.
    // On the async runtime that would block a worker thread and stall unrelated
    // requests, so it goes to the blocking pool.
    let decoded =
        tokio::task::spawn_blocking(move || decode_image(&bytes).map(|format| (format, bytes)))
            .await;

    let (format, bytes) = match decoded {
        Ok(Ok(result)) => result,
        Ok(Err(response)) => return *response,
        Err(error) => return internal_error("joining the image decode task", error),
    };

    let filename = format!("{}.{}", Uuid::new_v4(), extension_for(format));
    let destination = state.avatars_dir.join(&filename);

    if let Err(error) = tokio::fs::write(&destination, &bytes).await {
        return internal_error("writing the avatar file", error);
    }

    let relative = format!("{AVATARS_DIRNAME}/{filename}");

    // The previous file is read and removed *after* the row is updated, not
    // before. The spec for this feature said before; the order matters, and
    // after is the safe one: if the update fails between the two, deleting
    // first leaves a row pointing at a file that no longer exists — a broken
    // avatar with no way to notice — whereas deleting after can at worst leave
    // one orphaned file, which is inert. Neither order accumulates files in the
    // normal case, which is what the rule is for.
    let previous = match replace_avatar_path(&state, caller.id(), Some(&relative)).await {
        Ok(previous) => previous,
        Err(response) => {
            // The row was not updated, so nothing references the file we just
            // wrote. Remove it rather than leaving it behind.
            let _ = tokio::fs::remove_file(&destination).await;
            return *response;
        }
    };

    remove_avatar_file(&state.avatars_dir, previous.as_deref()).await;

    Json(AvatarResponse {
        avatar_url: format!("/{relative}"),
    })
    .into_response()
}

/// `DELETE /account/avatar`
pub async fn delete_avatar(caller: AuthenticatedUser, State(state): State<AppState>) -> Response {
    let previous = match replace_avatar_path(&state, caller.id(), None).await {
        Ok(previous) => previous,
        Err(response) => return *response,
    };

    remove_avatar_file(&state.avatars_dir, previous.as_deref()).await;

    let mut conn = match state.pool.acquire().await {
        Ok(conn) => conn,
        Err(error) => return internal_error("acquiring a connection", error),
    };

    match load_account(&mut conn, caller.id()).await {
        Ok(Some(account)) => Json(account).into_response(),
        Ok(None) => account_gone(),
        Err(error) => internal_error("reloading the account", error),
    }
}

/// Pulls the single file part out of a multipart body.
///
/// Takes the first field carrying a filename. A field without one is a plain
/// form value, not an upload, and skipping those means a client that also sends
/// text fields is not rejected for it.
async fn read_upload(mut multipart: Multipart) -> Result<Vec<u8>, Box<Response>> {
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => {
                return Err(Box::new(ErrorBody::message(
                    StatusCode::BAD_REQUEST,
                    "expected a file field in the multipart body".to_owned(),
                )))
            }
            Err(error) => {
                // Includes a body that exceeded the limit, which the layer
                // below reports as a multipart read failure.
                return Err(Box::new(ErrorBody::message(
                    StatusCode::BAD_REQUEST,
                    format!("could not read the upload: {error}"),
                )));
            }
        };

        if field.file_name().is_none() {
            continue;
        }

        return match field.bytes().await {
            Ok(bytes) => Ok(bytes.to_vec()),
            Err(error) => Err(Box::new(ErrorBody::message(
                StatusCode::BAD_REQUEST,
                format!("could not read the uploaded file: {error}"),
            ))),
        };
    }
}

/// Confirms bytes are a decodable image in an accepted format.
///
/// The declared `Content-Type` is not consulted at any point. It is a string
/// the caller chose, so a `.exe` announced as `image/png` would pass a
/// header check; only decoding tells you what the bytes are. The format is
/// taken from the content's own magic bytes, and the image is decoded in full
/// rather than merely having its header parsed — a truncated or malformed file
/// has a perfectly valid header.
fn decode_image(bytes: &[u8]) -> Result<ImageFormat, Box<Response>> {
    let reader = match image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() {
        Ok(reader) => reader,
        Err(error) => {
            return Err(Box::new(ErrorBody::message(
                StatusCode::BAD_REQUEST,
                format!("could not read the uploaded file: {error}"),
            )))
        }
    };

    let format = match reader.format() {
        Some(format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)) => format,
        // Recognised, but not one of ours. Named so the user knows what to
        // convert from rather than guessing.
        Some(other) => {
            return Err(Box::new(ErrorBody::message(
                StatusCode::BAD_REQUEST,
                format!(
                    "avatar must be a PNG, JPEG, or WebP image (this looks like {})",
                    format_name(other)
                ),
            )))
        }
        None => {
            return Err(Box::new(ErrorBody::message(
                StatusCode::BAD_REQUEST,
                "avatar must be a PNG, JPEG, or WebP image".to_owned(),
            )))
        }
    };

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);

    let mut reader = reader;
    reader.limits(limits);

    match reader.decode() {
        Ok(_) => Ok(format),
        // Covers a truncated file, a corrupt one, and a decompression bomb that
        // tripped the allocation ceiling. All three are the caller's file being
        // unusable, which is a 400.
        Err(error) => Err(Box::new(ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("the uploaded file is not a usable image: {error}"),
        ))),
    }
}

/// The extension stored for a format. Fixed per format rather than taken from
/// the upload's filename, which is caller-controlled.
fn extension_for(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::WebP => "webp",
        // Unreachable: `decode_image` returns only the three above. Kept total
        // rather than panicking, so adding a format to that match cannot turn
        // into a crash here.
        _ => "bin",
    }
}

/// A human name for a rejected format.
fn format_name(format: ImageFormat) -> String {
    format
        .extensions_str()
        .first()
        .map(|name| name.to_uppercase())
        .unwrap_or_else(|| "an unsupported format".to_owned())
}

/// Points a user's `avatar_path` at `next`, returning what it pointed at before.
///
/// Read and write in one transaction, because the caller uses the returned
/// value to decide which file to delete. Reading outside the transaction would
/// leave a window in which two concurrent uploads both observe the same old
/// path, and the slower one deletes a file the faster one has already replaced
/// — leaving a row pointing at nothing.
///
/// Note that `RETURNING` cannot do this job: SQLite evaluates it against the
/// row as it exists *after* the update, so it yields the new path, not the old.
async fn replace_avatar_path(
    state: &AppState,
    user_id: &str,
    next: Option<&str>,
) -> Result<Option<String>, Box<Response>> {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            return Err(Box::new(internal_error(
                "beginning the avatar transaction",
                error,
            )))
        }
    };

    let existing =
        sqlx::query_as::<_, (Option<String>,)>("SELECT avatar_path FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await;

    let previous = match existing {
        Ok(Some((previous,))) => previous,
        Ok(None) => return Err(Box::new(account_gone())),
        Err(error) => {
            return Err(Box::new(internal_error(
                "reading the current avatar",
                error,
            )))
        }
    };

    if let Err(error) = sqlx::query("UPDATE users SET avatar_path = ? WHERE id = ?")
        .bind(next)
        .bind(user_id)
        .execute(&mut *tx)
        .await
    {
        return Err(Box::new(internal_error("updating the avatar path", error)));
    }

    if let Err(error) = tx.commit().await {
        return Err(Box::new(internal_error(
            "committing the avatar change",
            error,
        )));
    }

    Ok(previous)
}

/// Deletes a stored avatar file, if there is one.
///
/// Best-effort: a file that is already gone, or that cannot be removed, is not
/// worth failing a request over — the row no longer references it either way.
///
/// The path is rebuilt from the directory plus the stored *file name*, never by
/// joining the stored string directly. These values are ours, generated from a
/// UUID, so today that is belt-and-braces; it stays correct if a path ever
/// reaches the column from somewhere less trustworthy, where a `..` segment
/// would otherwise let a delete escape the avatar directory.
async fn remove_avatar_file(avatars_dir: &Path, stored_path: Option<&str>) {
    let Some(stored) = stored_path else { return };
    let Some(name) = PathBuf::from(stored)
        .file_name()
        .map(std::ffi::OsString::from)
    else {
        return;
    };

    let path = avatars_dir.join(name);
    if let Err(error) = tokio::fs::remove_file(&path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), %error, "could not remove a replaced avatar");
        }
    }
}

/// Loads a profile with its group memberships.
async fn load_account(
    conn: &mut SqliteConnection,
    user_id: &str,
) -> Result<Option<AccountResponse>, sqlx::Error> {
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT id, username, display_name, avatar_path, created_at FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else { return Ok(None) };

    let groups = sqlx::query_as::<_, (String, String)>(
        "SELECT g.id, g.name FROM groups g \
         JOIN user_groups ug ON ug.group_id = g.id \
         WHERE ug.user_id = ? ORDER BY g.name",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(Some(AccountResponse {
        id: row.id,
        username: row.username,
        display_name: row.display_name,
        avatar_url: row.avatar_path.map(|path| format!("/{path}")),
        created_at: row.created_at,
        groups: groups
            .into_iter()
            .map(|(id, name)| AccountGroup { id, name })
            .collect(),
    }))
}

/// The account behind a valid token no longer exists — deleted mid-session.
fn account_gone() -> Response {
    ErrorBody::message(
        StatusCode::NOT_FOUND,
        "this account no longer exists".to_owned(),
    )
}
