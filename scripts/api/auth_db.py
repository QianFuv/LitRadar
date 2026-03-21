"""User authentication database (SQLite)."""

from __future__ import annotations

import hashlib
import hmac
import json
import secrets
import sqlite3
import time

from scripts.shared.constants import PROJECT_ROOT

AUTH_DB_PATH = PROJECT_ROOT / "data" / "auth.sqlite"
ACCESS_TOKEN_BYTES = 32
ACCESS_TOKEN_DEFAULT_TTL = 7 * 24 * 3600


def _get_connection() -> sqlite3.Connection:
    """Open a connection to the auth SQLite database."""
    AUTH_DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(AUTH_DB_PATH))
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def init_auth_db() -> None:
    """Create auth tables if they don't exist."""
    conn = _get_connection()
    try:
        conn.executescript(
            """
            CREATE TABLE IF NOT EXISTS users (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                username    TEXT    NOT NULL UNIQUE COLLATE NOCASE,
                password_hash TEXT  NOT NULL,
                salt        TEXT    NOT NULL,
                is_admin    INTEGER NOT NULL DEFAULT 0,
                created_at  REAL   NOT NULL,
                updated_at  REAL   NOT NULL
            );

            CREATE TABLE IF NOT EXISTS access_tokens (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash  TEXT    NOT NULL UNIQUE,
                name        TEXT    NOT NULL DEFAULT '',
                expires_at  REAL   NOT NULL,
                created_at  REAL   NOT NULL
            );

            CREATE TABLE IF NOT EXISTS folders (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name        TEXT    NOT NULL,
                is_tracking INTEGER NOT NULL DEFAULT 0,
                created_at  REAL   NOT NULL,
                updated_at  REAL   NOT NULL,
                UNIQUE(user_id, name)
            );

            CREATE TABLE IF NOT EXISTS favorites (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                folder_id   INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                article_id  INTEGER NOT NULL,
                db_name     TEXT    NOT NULL DEFAULT '',
                note        TEXT    NOT NULL DEFAULT '',
                created_at  REAL   NOT NULL,
                UNIQUE(user_id, folder_id, article_id, db_name)
            );

            CREATE TABLE IF NOT EXISTS invite_codes (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                code        TEXT    NOT NULL UNIQUE,
                created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
                used_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
                used_at     REAL,
                created_at  REAL   NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_access_tokens_user
                ON access_tokens(user_id);
            CREATE INDEX IF NOT EXISTS idx_folders_user
                ON folders(user_id);
            CREATE INDEX IF NOT EXISTS idx_favorites_folder
                ON favorites(folder_id);
            CREATE INDEX IF NOT EXISTS idx_favorites_user
                ON favorites(user_id);
            CREATE INDEX IF NOT EXISTS idx_invite_codes_code
                ON invite_codes(code);
            CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by
                ON invite_codes(created_by);

            CREATE TABLE IF NOT EXISTS notification_settings (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id         INTEGER NOT NULL UNIQUE
                                    REFERENCES users(id) ON DELETE CASCADE,
                keywords        TEXT    NOT NULL DEFAULT '[]',
                directions      TEXT    NOT NULL DEFAULT '[]',
                delivery_method TEXT    NOT NULL DEFAULT 'folder',
                pushplus_token  TEXT    NOT NULL DEFAULT '',
                pushplus_template TEXT  NOT NULL DEFAULT 'markdown',
                pushplus_topic  TEXT    NOT NULL DEFAULT '',
                pushplus_channel TEXT   NOT NULL DEFAULT 'wechat',
                sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0,
                ai_base_url     TEXT    NOT NULL DEFAULT '',
                ai_api_key      TEXT    NOT NULL DEFAULT '',
                ai_model        TEXT    NOT NULL DEFAULT '',
                ai_system_prompt TEXT   NOT NULL DEFAULT '',
                ai_backup_base_url TEXT NOT NULL DEFAULT '',
                ai_backup_api_key TEXT NOT NULL DEFAULT '',
                ai_backup_model TEXT NOT NULL DEFAULT '',
                ai_backup_system_prompt TEXT NOT NULL DEFAULT '',
                ai_retry_attempts INTEGER NOT NULL DEFAULT 3,
                enabled         INTEGER NOT NULL DEFAULT 1,
                created_at      REAL    NOT NULL,
                updated_at      REAL    NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_notification_settings_user
                ON notification_settings(user_id);

            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                name            TEXT    NOT NULL,
                command         TEXT    NOT NULL,
                cron            TEXT    NOT NULL,
                enabled         INTEGER NOT NULL DEFAULT 1,
                last_run_at     REAL,
                last_status     TEXT    NOT NULL DEFAULT '',
                created_at      REAL    NOT NULL,
                updated_at      REAL    NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_enabled
                ON scheduled_tasks(enabled);

            CREATE TABLE IF NOT EXISTS announcements (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                title           TEXT    NOT NULL,
                message         TEXT    NOT NULL,
                priority        TEXT    NOT NULL DEFAULT 'normal',
                enabled         INTEGER NOT NULL DEFAULT 1,
                created_at      REAL    NOT NULL,
                updated_at      REAL    NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_announcements_enabled
                ON announcements(enabled);
            """
        )
        conn.commit()

        cols = {row[1] for row in conn.execute("PRAGMA table_info(users)").fetchall()}
        if "is_admin" not in cols:
            conn.execute(
                "ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0"
            )
            conn.execute(
                "UPDATE users SET is_admin = 1 WHERE id = (SELECT MIN(id) FROM users)"
            )
            conn.commit()

        notification_cols = {
            row[1]
            for row in conn.execute(
                "PRAGMA table_info(notification_settings)"
            ).fetchall()
        }
        notification_migrations = {
            "ai_base_url": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_base_url TEXT NOT NULL DEFAULT ''"
            ),
            "ai_api_key": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_api_key TEXT NOT NULL DEFAULT ''"
            ),
            "ai_model": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_model TEXT NOT NULL DEFAULT ''"
            ),
            "ai_system_prompt": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_system_prompt TEXT NOT NULL DEFAULT ''"
            ),
            "ai_backup_base_url": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_backup_base_url TEXT NOT NULL DEFAULT ''"
            ),
            "ai_backup_api_key": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_backup_api_key TEXT NOT NULL DEFAULT ''"
            ),
            "ai_backup_model": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_backup_model TEXT NOT NULL DEFAULT ''"
            ),
            "ai_backup_system_prompt": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_backup_system_prompt TEXT NOT NULL DEFAULT ''"
            ),
            "ai_retry_attempts": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN ai_retry_attempts INTEGER NOT NULL DEFAULT 3"
            ),
            "sync_to_tracking_folder": (
                "ALTER TABLE notification_settings "
                "ADD COLUMN sync_to_tracking_folder INTEGER NOT NULL DEFAULT 0"
            ),
        }
        for column_name, statement in notification_migrations.items():
            if column_name not in notification_cols:
                conn.execute(statement)

        announcement_cols = {
            row[1]
            for row in conn.execute("PRAGMA table_info(announcements)").fetchall()
        }
        if announcement_cols and "priority" not in announcement_cols:
            conn.execute(
                "ALTER TABLE announcements "
                "ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal'"
            )
        purge_expired_access_tokens(conn=conn)
        conn.commit()
    finally:
        conn.close()


def _hash_password(password: str, salt: str) -> str:
    """Hash a password with salt using PBKDF2-HMAC-SHA256."""
    return hashlib.pbkdf2_hmac(
        "sha256", password.encode(), salt.encode(), iterations=260_000
    ).hex()


def _hash_token(token: str) -> str:
    """Hash an access token using SHA-256."""
    return hashlib.sha256(token.encode()).hexdigest()


def purge_expired_access_tokens(
    *,
    now: float | None = None,
    conn: sqlite3.Connection | None = None,
) -> int:
    """
    Delete expired access tokens from the auth database.

    Args:
        now: Optional current timestamp override.
        conn: Optional existing SQLite connection.

    Returns:
        Number of deleted token rows.
    """
    current_time = time.time() if now is None else now
    connection = conn or _get_connection()
    owns_connection = conn is None
    try:
        cur = connection.execute(
            "DELETE FROM access_tokens WHERE expires_at <= ?",
            (current_time,),
        )
        if owns_connection:
            connection.commit()
        return cur.rowcount
    finally:
        if owns_connection:
            connection.close()


def create_user(username: str, password: str) -> dict:
    """
    Create a new user with a default favorites/tracking folder.

    Returns:
        User dict with id, username, created_at.
    """
    salt = secrets.token_hex(16)
    now = time.time()
    password_hash = _hash_password(password, salt)
    conn = _get_connection()
    try:
        conn.execute(
            "INSERT INTO users (username, password_hash, salt, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?)",
            (username, password_hash, salt, now, now),
        )
        row = conn.execute(
            "SELECT id, username, created_at FROM users WHERE username = ?",
            (username,),
        ).fetchone()
        user_id = row["id"]
        conn.execute(
            "INSERT INTO folders (user_id, name, is_tracking, created_at, updated_at) "
            "VALUES (?, ?, 1, ?, ?)",
            (user_id, "默认收藏", now, now),
        )
        conn.commit()
        return dict(row)
    finally:
        conn.close()


def register_with_invite(
    username: str, password: str, invite_code: str | None = None
) -> dict:
    """
    Atomically create a user, enforce the invite requirement, and consume the
    invite code — all inside a single SQLite transaction.

    The first-user check ("no invite needed") is performed inside the
    transaction so two concurrent requests on an empty database cannot both
    skip the invite requirement.

    The user row is inserted first, then the invite code is consumed with a
    single ``UPDATE … WHERE used_by IS NULL`` using the real ``user_id`` so
    no foreign-key placeholder is needed.  If the code turns out to be
    invalid or already consumed the whole transaction is rolled back —
    neither the user nor the folder are persisted.

    Args:
        username: Desired username.
        password: Plain-text password (will be salted + hashed).
        invite_code: Invite code string, or None / empty.

    Returns:
        User dict with id, username, created_at.

    Raises:
        ValueError: If an invite code is required but missing, or invalid /
            already consumed.
        sqlite3.IntegrityError: If the username already exists.
    """
    salt = secrets.token_hex(16)
    now = time.time()
    password_hash = _hash_password(password, salt)
    conn = _get_connection()
    try:
        conn.execute("BEGIN IMMEDIATE")
        user_count = conn.execute("SELECT COUNT(*) FROM users").fetchone()[0]
        needs_invite = user_count > 0

        if needs_invite and not invite_code:
            raise ValueError("Invite code is required")

        is_admin = 1 if not needs_invite else 0
        conn.execute(
            "INSERT INTO users "
            "(username, password_hash, salt, is_admin, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (username, password_hash, salt, is_admin, now, now),
        )
        row = conn.execute(
            "SELECT id, username, is_admin, created_at FROM users WHERE username = ?",
            (username,),
        ).fetchone()
        user_id = row["id"]

        if needs_invite:
            cur = conn.execute(
                "UPDATE invite_codes SET used_by = ?, used_at = ? "
                "WHERE code = ? AND used_by IS NULL",
                (user_id, now, invite_code),
            )
            if cur.rowcount == 0:
                raise ValueError("Invalid or used invite code")

        conn.execute(
            "INSERT INTO folders "
            "(user_id, name, is_tracking, created_at, updated_at) "
            "VALUES (?, ?, 1, ?, ?)",
            (user_id, "默认收藏", now, now),
        )
        conn.commit()
        return dict(row)
    except BaseException:
        conn.rollback()
        raise
    finally:
        conn.close()


def verify_user(username: str, password: str) -> dict | None:
    """Verify credentials. Returns user dict or None."""
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, username, password_hash, salt, is_admin, created_at"
            " FROM users WHERE username = ?",
            (username,),
        ).fetchone()
        if not row:
            return None
        expected = _hash_password(password, row["salt"])
        if not hmac.compare_digest(expected, row["password_hash"]):
            return None
        return {
            "id": row["id"],
            "username": row["username"],
            "is_admin": bool(row["is_admin"]),
            "created_at": row["created_at"],
        }
    finally:
        conn.close()


def get_user_by_id(user_id: int) -> dict | None:
    """Get a user by ID. Returns user dict or None."""
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, username, created_at FROM users WHERE id = ?",
            (user_id,),
        ).fetchone()
        return dict(row) if row else None
    finally:
        conn.close()


def change_password(user_id: int, old_password: str, new_password: str) -> bool:
    """Change password. Returns True on success."""
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT password_hash, salt FROM users WHERE id = ?", (user_id,)
        ).fetchone()
        if not row:
            return False
        if not hmac.compare_digest(
            _hash_password(old_password, row["salt"]), row["password_hash"]
        ):
            return False
        new_salt = secrets.token_hex(16)
        new_hash = _hash_password(new_password, new_salt)
        conn.execute(
            "UPDATE users SET password_hash = ?, salt = ?, updated_at = ? WHERE id = ?",
            (new_hash, new_salt, time.time(), user_id),
        )
        conn.execute(
            "DELETE FROM access_tokens WHERE user_id = ?",
            (user_id,),
        )
        conn.commit()
        return True
    finally:
        conn.close()


def create_access_token(
    user_id: int, name: str = "", ttl: int = ACCESS_TOKEN_DEFAULT_TTL
) -> dict:
    """Create an access token. Returns {token, id, name, expires_at}."""
    raw_token = secrets.token_urlsafe(ACCESS_TOKEN_BYTES)
    token_hash = _hash_token(raw_token)
    now = time.time()
    expires_at = now + ttl
    conn = _get_connection()
    try:
        purge_expired_access_tokens(now=now, conn=conn)
        cur = conn.execute(
            "INSERT INTO access_tokens"
            " (user_id, token_hash, name, expires_at, created_at)"
            " VALUES (?, ?, ?, ?, ?)",
            (user_id, token_hash, name, expires_at, now),
        )
        conn.commit()
        return {
            "id": cur.lastrowid,
            "token": raw_token,
            "name": name,
            "expires_at": expires_at,
        }
    finally:
        conn.close()


def verify_access_token(raw_token: str) -> dict | None:
    """Verify an access token. Returns user dict or None."""
    token_hash = _hash_token(raw_token)
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT t.user_id, t.expires_at, u.id, u.username, u.is_admin "
            "FROM access_tokens t JOIN users u ON t.user_id = u.id "
            "WHERE t.token_hash = ?",
            (token_hash,),
        ).fetchone()
        if not row:
            return None
        if row["expires_at"] < time.time():
            conn.execute(
                "DELETE FROM access_tokens WHERE token_hash = ?",
                (token_hash,),
            )
            conn.commit()
            return None
        return {
            "id": row["user_id"],
            "username": row["username"],
            "is_admin": bool(row["is_admin"]),
        }
    finally:
        conn.close()


def list_access_tokens(user_id: int) -> list[dict]:
    """List non-expired tokens for a user (without the raw token)."""
    conn = _get_connection()
    try:
        current_time = time.time()
        purge_expired_access_tokens(now=current_time, conn=conn)
        rows = conn.execute(
            "SELECT id, name, expires_at, created_at FROM access_tokens "
            "WHERE user_id = ? AND expires_at > ? ORDER BY created_at DESC",
            (user_id, current_time),
        ).fetchall()
        conn.commit()
        return [dict(r) for r in rows]
    finally:
        conn.close()


def revoke_access_token(user_id: int, token_id: int) -> bool:
    """Revoke an access token by ID. Returns True if deleted."""
    conn = _get_connection()
    try:
        cur = conn.execute(
            "DELETE FROM access_tokens WHERE id = ? AND user_id = ?",
            (token_id, user_id),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def revoke_access_token_value(raw_token: str) -> bool:
    """
    Revoke an access token by its raw bearer token value.

    Args:
        raw_token: Raw bearer token string.

    Returns:
        True if a token row was deleted.
    """
    token_hash = _hash_token(raw_token)
    conn = _get_connection()
    try:
        cur = conn.execute(
            "DELETE FROM access_tokens WHERE token_hash = ?",
            (token_hash,),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def create_folder(user_id: int, name: str, is_tracking: bool = False) -> dict:
    """Create a new folder for the user.

    If *is_tracking* is True the existing tracking folder (if any) is
    cleared first so that the invariant of at most one tracking folder
    per user is preserved.
    """
    now = time.time()
    conn = _get_connection()
    try:
        if is_tracking:
            conn.execute(
                "UPDATE folders SET is_tracking = 0, updated_at = ? WHERE user_id = ?",
                (now, user_id),
            )
        cur = conn.execute(
            "INSERT INTO folders (user_id, name, is_tracking, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?)",
            (user_id, name, int(is_tracking), now, now),
        )
        conn.commit()
        return {
            "id": cur.lastrowid,
            "name": name,
            "is_tracking": is_tracking,
            "created_at": now,
        }
    finally:
        conn.close()


def list_folders(user_id: int) -> list[dict]:
    """List all folders for a user with article counts."""
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT f.id, f.name, f.is_tracking, f.created_at, f.updated_at, "
            "COUNT(fav.id) AS article_count "
            "FROM folders f LEFT JOIN favorites fav ON fav.folder_id = f.id "
            "WHERE f.user_id = ? GROUP BY f.id ORDER BY f.created_at",
            (user_id,),
        ).fetchall()
        return [dict(r) for r in rows]
    finally:
        conn.close()


def rename_folder(user_id: int, folder_id: int, new_name: str) -> bool:
    """Rename a folder. Returns True if updated."""
    conn = _get_connection()
    try:
        cur = conn.execute(
            "UPDATE folders SET name = ?, updated_at = ? WHERE id = ? AND user_id = ?",
            (new_name, time.time(), folder_id, user_id),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def delete_folder(user_id: int, folder_id: int) -> bool:
    """Delete a folder by ID. Returns True if deleted."""
    conn = _get_connection()
    try:
        cur = conn.execute(
            "DELETE FROM folders WHERE id = ? AND user_id = ?",
            (folder_id, user_id),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def get_tracking_folder(user_id: int) -> dict | None:
    """Get the tracking folder for a user (auto-receive weekly pushes)."""
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, name, is_tracking, created_at FROM folders "
            "WHERE user_id = ? AND is_tracking = 1 LIMIT 1",
            (user_id,),
        ).fetchone()
        return dict(row) if row else None
    finally:
        conn.close()


def set_tracking_folder(user_id: int, folder_id: int) -> bool:
    """Set a folder as the tracking folder (only one per user).

    The target folder is verified *before* the existing flag is cleared
    so that a stale/invalid folder_id does not wipe the current
    tracking setting.
    """
    conn = _get_connection()
    try:
        target = conn.execute(
            "SELECT id FROM folders WHERE id = ? AND user_id = ?",
            (folder_id, user_id),
        ).fetchone()
        if not target:
            return False
        now = time.time()
        conn.execute(
            "UPDATE folders SET is_tracking = 0, updated_at = ? WHERE user_id = ?",
            (now, user_id),
        )
        conn.execute(
            "UPDATE folders SET is_tracking = 1, updated_at = ?"
            " WHERE id = ? AND user_id = ?",
            (now, folder_id, user_id),
        )
        conn.commit()
        return True
    finally:
        conn.close()


def add_favorite(
    user_id: int, folder_id: int, article_id: int, db_name: str, note: str = ""
) -> dict:
    """Add an article to a folder as a favorite."""
    now = time.time()
    conn = _get_connection()
    try:
        folder = conn.execute(
            "SELECT id FROM folders WHERE id = ? AND user_id = ?",
            (folder_id, user_id),
        ).fetchone()
        if not folder:
            raise ValueError("Folder not found")
        cur = conn.execute(
            "INSERT OR IGNORE INTO favorites"
            " (user_id, folder_id, article_id, db_name, note, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            (user_id, folder_id, article_id, db_name, note, now),
        )
        conn.commit()
        return {
            "id": cur.lastrowid,
            "folder_id": folder_id,
            "article_id": article_id,
            "db_name": db_name,
            "note": note,
            "created_at": now,
        }
    finally:
        conn.close()


def remove_favorite(
    user_id: int, folder_id: int, article_id: int, db_name: str
) -> bool:
    """Remove a favorite. Returns True if deleted."""
    conn = _get_connection()
    try:
        cur = conn.execute(
            "DELETE FROM favorites WHERE user_id = ? AND folder_id = ? "
            "AND article_id = ? AND db_name = ?",
            (user_id, folder_id, article_id, db_name),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def list_favorites(
    user_id: int, folder_id: int | None = None, limit: int = 100, offset: int = 0
) -> list[dict]:
    """List favorites for a user, optionally filtered by folder."""
    conn = _get_connection()
    try:
        if folder_id is not None:
            rows = conn.execute(
                "SELECT id, folder_id, article_id, db_name, note, created_at "
                "FROM favorites WHERE user_id = ? AND folder_id = ? "
                "ORDER BY created_at DESC LIMIT ? OFFSET ?",
                (user_id, folder_id, limit, offset),
            ).fetchall()
        else:
            rows = conn.execute(
                "SELECT id, folder_id, article_id, db_name, note, created_at "
                "FROM favorites WHERE user_id = ? "
                "ORDER BY created_at DESC LIMIT ? OFFSET ?",
                (user_id, limit, offset),
            ).fetchall()
        return [dict(r) for r in rows]
    finally:
        conn.close()


def count_favorites(user_id: int, folder_id: int | None = None) -> int:
    """Count favorites for a user, optionally filtered by folder."""
    conn = _get_connection()
    try:
        if folder_id is not None:
            row = conn.execute(
                "SELECT COUNT(*) AS cnt FROM favorites"
                " WHERE user_id = ? AND folder_id = ?",
                (user_id, folder_id),
            ).fetchone()
        else:
            row = conn.execute(
                "SELECT COUNT(*) AS cnt FROM favorites WHERE user_id = ?",
                (user_id,),
            ).fetchone()
        return row["cnt"]
    finally:
        conn.close()


def is_favorited(user_id: int, article_id: int, db_name: str) -> list[dict]:
    """Check which folders an article is favorited in."""
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT fav.folder_id, f.name AS folder_name "
            "FROM favorites fav JOIN folders f ON fav.folder_id = f.id "
            "WHERE fav.user_id = ? AND fav.article_id = ? AND fav.db_name = ?",
            (user_id, article_id, db_name),
        ).fetchall()
        return [dict(r) for r in rows]
    finally:
        conn.close()


def batch_is_favorited(
    user_id: int,
    article_ids: list[int],
    db_name: str,
) -> dict[int, list[dict]]:
    """
    Check which folders multiple articles are favorited in.

    Args:
        user_id: User identifier.
        article_ids: Article identifiers to check.
        db_name: Database name for the favorite rows.

    Returns:
        Mapping of article_id to folder membership rows.
    """
    unique_article_ids = list(dict.fromkeys(article_ids))
    if not unique_article_ids:
        return {}

    conn = _get_connection()
    try:
        placeholders = ", ".join("?" for _ in unique_article_ids)
        rows = conn.execute(
            f"""
            SELECT fav.article_id, fav.folder_id, f.name AS folder_name
            FROM favorites fav
            JOIN folders f ON fav.folder_id = f.id
            WHERE fav.user_id = ?
              AND fav.db_name = ?
              AND fav.article_id IN ({placeholders})
            ORDER BY fav.article_id, fav.created_at
            """,
            (user_id, db_name, *unique_article_ids),
        ).fetchall()
        result: dict[int, list[dict]] = {
            article_id: [] for article_id in unique_article_ids
        }
        for row in rows:
            item = dict(row)
            result[int(item["article_id"])].append(
                {
                    "folder_id": item["folder_id"],
                    "folder_name": item["folder_name"],
                }
            )
        return result
    finally:
        conn.close()


def _normalize_favorite_articles(articles: list[dict]) -> list[tuple[int, str]]:
    """
    Normalize favorite article references for bulk operations.

    Args:
        articles: Raw article mappings.

    Returns:
        Deduplicated ``(article_id, db_name)`` tuples in input order.
    """
    normalized: list[tuple[int, str]] = []
    seen: set[tuple[int, str]] = set()
    for article in articles:
        article_id = article.get("article_id")
        if not isinstance(article_id, int) or article_id <= 0:
            continue
        db_name = str(article.get("db_name") or "")
        key = (article_id, db_name)
        if key in seen:
            continue
        seen.add(key)
        normalized.append(key)
    return normalized


def bulk_add_favorites(user_id: int, folder_id: int, articles: list[dict]) -> int:
    """Bulk add favorites. Returns count of inserted rows."""
    now = time.time()
    conn = _get_connection()
    try:
        folder = conn.execute(
            "SELECT id FROM folders WHERE id = ? AND user_id = ?",
            (folder_id, user_id),
        ).fetchone()
        if not folder:
            raise ValueError("Folder not found")
        rows = [
            (
                user_id,
                folder_id,
                a["article_id"],
                a.get("db_name", ""),
                a.get("note", ""),
                now,
            )
            for a in articles
        ]
        cur = conn.executemany(
            "INSERT OR IGNORE INTO favorites"
            " (user_id, folder_id, article_id, db_name, note, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            rows,
        )
        conn.commit()
        return cur.rowcount
    finally:
        conn.close()


def bulk_remove_favorites(user_id: int, folder_id: int, articles: list[dict]) -> int:
    """
    Bulk remove favorites from one folder.

    Args:
        user_id: User identifier.
        folder_id: Source folder identifier.
        articles: Raw favorite article references.

    Returns:
        Number of deleted favorite rows.

    Raises:
        ValueError: If the folder does not belong to the user.
    """
    normalized_articles = _normalize_favorite_articles(articles)
    if not normalized_articles:
        return 0

    conn = _get_connection()
    try:
        folder = conn.execute(
            "SELECT id FROM folders WHERE id = ? AND user_id = ?",
            (folder_id, user_id),
        ).fetchone()
        if not folder:
            raise ValueError("Folder not found")

        before_changes = conn.total_changes
        conn.executemany(
            "DELETE FROM favorites WHERE user_id = ? AND folder_id = ? "
            "AND article_id = ? AND db_name = ?",
            [
                (user_id, folder_id, article_id, db_name)
                for article_id, db_name in normalized_articles
            ],
        )
        conn.commit()
        return conn.total_changes - before_changes
    finally:
        conn.close()


def bulk_move_favorites(
    user_id: int,
    source_folder_id: int,
    target_folder_id: int,
    articles: list[dict],
) -> int:
    """
    Move favorites from one folder to another.

    Args:
        user_id: User identifier.
        source_folder_id: Folder to move favorites from.
        target_folder_id: Folder to move favorites into.
        articles: Raw favorite article references.

    Returns:
        Number of source rows removed during the move.

    Raises:
        ValueError: If the folders are invalid or identical.
    """
    if source_folder_id == target_folder_id:
        raise ValueError("Source and target folders must be different")

    normalized_articles = _normalize_favorite_articles(articles)
    if not normalized_articles:
        return 0

    now = time.time()
    conn = _get_connection()
    try:
        source_folder = conn.execute(
            "SELECT id FROM folders WHERE id = ? AND user_id = ?",
            (source_folder_id, user_id),
        ).fetchone()
        if not source_folder:
            raise ValueError("Source folder not found")

        target_folder = conn.execute(
            "SELECT id FROM folders WHERE id = ? AND user_id = ?",
            (target_folder_id, user_id),
        ).fetchone()
        if not target_folder:
            raise ValueError("Target folder not found")

        insert_rows = [
            (
                target_folder_id,
                now,
                user_id,
                source_folder_id,
                article_id,
                db_name,
            )
            for article_id, db_name in normalized_articles
        ]
        delete_rows = [
            (user_id, source_folder_id, article_id, db_name)
            for article_id, db_name in normalized_articles
        ]

        conn.execute("BEGIN")
        conn.executemany(
            "INSERT OR IGNORE INTO favorites "
            " (user_id, folder_id, article_id, db_name, note, created_at) "
            "SELECT user_id, ?, article_id, db_name, note, ? "
            "FROM favorites WHERE user_id = ? AND folder_id = ? "
            "AND article_id = ? AND db_name = ?",
            insert_rows,
        )
        before_delete = conn.total_changes
        conn.executemany(
            "DELETE FROM favorites WHERE user_id = ? AND folder_id = ? "
            "AND article_id = ? AND db_name = ?",
            delete_rows,
        )
        conn.commit()
        return conn.total_changes - before_delete
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()


INVITE_CODE_BYTES = 8


def create_invite_code(user_id: int) -> dict:
    """
    Generate an invite code for a user. Each user can create at most one code.

    Args:
        user_id: The user who generates the code.

    Returns:
        Dict with id, code, created_at.

    Raises:
        ValueError: If the user has already generated an invite code.
    """
    conn = _get_connection()
    try:
        existing = conn.execute(
            "SELECT id FROM invite_codes WHERE created_by = ?",
            (user_id,),
        ).fetchone()
        if existing:
            raise ValueError("User has already generated an invite code")
        code = secrets.token_urlsafe(INVITE_CODE_BYTES)
        now = time.time()
        cur = conn.execute(
            "INSERT INTO invite_codes (code, created_by, created_at) VALUES (?, ?, ?)",
            (code, user_id, now),
        )
        conn.commit()
        return {"id": cur.lastrowid, "code": code, "created_at": now}
    finally:
        conn.close()


def consume_invite_code(code: str, user_id: int) -> bool:
    """
    Atomically verify and consume an invite code in a single UPDATE.

    Args:
        code: The invite code string.
        user_id: The user consuming the code.

    Returns:
        True if a valid unused code was consumed, False otherwise.
    """
    conn = _get_connection()
    try:
        cur = conn.execute(
            "UPDATE invite_codes SET used_by = ?, used_at = ? "
            "WHERE code = ? AND used_by IS NULL",
            (user_id, time.time(), code),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def verify_invite_code(code: str) -> bool:
    """
    Check if an invite code is valid (exists and unused).

    Args:
        code: The invite code string.

    Returns:
        True if the code is valid and available.
    """
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id FROM invite_codes WHERE code = ? AND used_by IS NULL",
            (code,),
        ).fetchone()
        return row is not None
    finally:
        conn.close()


def get_user_invite_code(user_id: int) -> dict | None:
    """
    Get the invite code created by a user, if any.

    Args:
        user_id: The user ID.

    Returns:
        Dict with code info or None.
    """
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, code, used_by, used_at, created_at "
            "FROM invite_codes WHERE created_by = ?",
            (user_id,),
        ).fetchone()
        return dict(row) if row else None
    finally:
        conn.close()


def count_users() -> int:
    """
    Count the total number of registered users.

    Returns:
        Number of users.
    """
    conn = _get_connection()
    try:
        row = conn.execute("SELECT COUNT(*) AS cnt FROM users").fetchone()
        return row["cnt"]
    finally:
        conn.close()


def get_notification_settings(user_id: int) -> dict | None:
    """
    Get notification settings for a user.

    Args:
        user_id: The user ID.

    Returns:
        Settings dict or None if not configured.
    """
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, user_id, keywords, directions, delivery_method, "
            "pushplus_token, pushplus_template, pushplus_topic, "
            "pushplus_channel, "
            "sync_to_tracking_folder, "
            "ai_base_url, ai_api_key, ai_model, ai_system_prompt, "
            "ai_backup_base_url, ai_backup_api_key, ai_backup_model, "
            "ai_backup_system_prompt, ai_retry_attempts, "
            "enabled, created_at, updated_at "
            "FROM notification_settings WHERE user_id = ?",
            (user_id,),
        ).fetchone()
        if not row:
            return None
        result = dict(row)
        result["keywords"] = json.loads(result["keywords"])
        result["directions"] = json.loads(result["directions"])
        result["sync_to_tracking_folder"] = bool(result["sync_to_tracking_folder"])
        result["ai_retry_attempts"] = max(1, int(result["ai_retry_attempts"]))
        result["enabled"] = bool(result["enabled"])
        return result
    finally:
        conn.close()


def upsert_notification_settings(
    user_id: int,
    keywords: list[str],
    directions: list[str],
    delivery_method: str,
    pushplus_token: str = "",
    pushplus_template: str = "markdown",
    pushplus_topic: str = "",
    pushplus_channel: str = "wechat",
    sync_to_tracking_folder: bool = False,
    ai_base_url: str = "",
    ai_api_key: str = "",
    ai_model: str = "",
    ai_system_prompt: str = "",
    ai_backup_base_url: str = "",
    ai_backup_api_key: str = "",
    ai_backup_model: str = "",
    ai_backup_system_prompt: str = "",
    ai_retry_attempts: int = 3,
    enabled: bool = True,
) -> dict:
    """
    Create or update notification settings for a user.

    Args:
        user_id: The user ID.
        keywords: Keyword preferences.
        directions: Research direction preferences.
        delivery_method: 'folder' or 'pushplus'.
        pushplus_token: PushPlus token.
        pushplus_template: PushPlus template.
        pushplus_topic: PushPlus topic.
        pushplus_channel: PushPlus channel override.
        sync_to_tracking_folder: Whether PushPlus also writes favorites.
        ai_base_url: OpenAI-compatible API base URL.
        ai_api_key: OpenAI-compatible API key.
        ai_model: OpenAI-compatible model name.
        ai_system_prompt: Optional custom system prompt.
        ai_backup_base_url: Backup OpenAI-compatible API base URL.
        ai_backup_api_key: Backup OpenAI-compatible API key.
        ai_backup_model: Backup OpenAI-compatible model name.
        ai_backup_system_prompt: Backup custom system prompt.
        ai_retry_attempts: Retry attempts per AI endpoint.
        enabled: Whether notifications are enabled.

    Returns:
        Updated settings dict.
    """
    now = time.time()
    keywords_json = json.dumps(keywords, ensure_ascii=False)
    directions_json = json.dumps(directions, ensure_ascii=False)
    conn = _get_connection()
    try:
        conn.execute(
            "INSERT INTO notification_settings "
            "(user_id, keywords, directions, delivery_method, "
            "pushplus_token, pushplus_template, pushplus_topic, "
            "pushplus_channel, sync_to_tracking_folder, "
            "ai_base_url, ai_api_key, ai_model, ai_system_prompt, "
            "ai_backup_base_url, ai_backup_api_key, ai_backup_model, "
            "ai_backup_system_prompt, ai_retry_attempts, "
            "enabled, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) "
            "ON CONFLICT(user_id) DO UPDATE SET "
            "keywords = excluded.keywords, "
            "directions = excluded.directions, "
            "delivery_method = excluded.delivery_method, "
            "pushplus_token = excluded.pushplus_token, "
            "pushplus_template = excluded.pushplus_template, "
            "pushplus_topic = excluded.pushplus_topic, "
            "pushplus_channel = excluded.pushplus_channel, "
            "sync_to_tracking_folder = excluded.sync_to_tracking_folder, "
            "ai_base_url = excluded.ai_base_url, "
            "ai_api_key = excluded.ai_api_key, "
            "ai_model = excluded.ai_model, "
            "ai_system_prompt = excluded.ai_system_prompt, "
            "ai_backup_base_url = excluded.ai_backup_base_url, "
            "ai_backup_api_key = excluded.ai_backup_api_key, "
            "ai_backup_model = excluded.ai_backup_model, "
            "ai_backup_system_prompt = excluded.ai_backup_system_prompt, "
            "ai_retry_attempts = excluded.ai_retry_attempts, "
            "enabled = excluded.enabled, "
            "updated_at = excluded.updated_at",
            (
                user_id,
                keywords_json,
                directions_json,
                delivery_method,
                pushplus_token,
                pushplus_template,
                pushplus_topic,
                pushplus_channel,
                int(sync_to_tracking_folder),
                ai_base_url,
                ai_api_key,
                ai_model,
                ai_system_prompt,
                ai_backup_base_url,
                ai_backup_api_key,
                ai_backup_model,
                ai_backup_system_prompt,
                ai_retry_attempts,
                int(enabled),
                now,
                now,
            ),
        )
        conn.commit()
    finally:
        conn.close()
    return get_notification_settings(user_id)  # type: ignore[return-value]


def list_notification_subscribers() -> list[dict]:
    """
    List all users with enabled notification settings.

    Returns:
        List of user notification settings with username.
    """
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT ns.id, ns.user_id, u.username, "
            "ns.keywords, ns.directions, ns.delivery_method, "
            "ns.pushplus_token, ns.pushplus_template, "
            "ns.pushplus_topic, ns.pushplus_channel, "
            "ns.sync_to_tracking_folder, "
            "ns.ai_base_url, ns.ai_api_key, ns.ai_model, ns.ai_system_prompt, "
            "ns.ai_backup_base_url, ns.ai_backup_api_key, ns.ai_backup_model, "
            "ns.ai_backup_system_prompt, ns.ai_retry_attempts, "
            "ns.enabled, ns.created_at, ns.updated_at "
            "FROM notification_settings ns "
            "JOIN users u ON ns.user_id = u.id "
            "WHERE ns.enabled = 1",
        ).fetchall()
        result = []
        for row in rows:
            item = dict(row)
            item["keywords"] = json.loads(item["keywords"])
            item["directions"] = json.loads(item["directions"])
            item["sync_to_tracking_folder"] = bool(item["sync_to_tracking_folder"])
            item["ai_retry_attempts"] = max(1, int(item["ai_retry_attempts"]))
            item["enabled"] = bool(item["enabled"])
            result.append(item)
        return result
    finally:
        conn.close()


def list_all_users() -> list[dict]:
    """
    List all users with stats (folder count, favorite count, has notifications).

    Returns:
        List of user info dicts.
    """
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT u.id, u.username, u.is_admin, "
            "u.created_at, u.updated_at, "
            "(SELECT COUNT(*) FROM folders f "
            "   WHERE f.user_id = u.id) AS folder_count, "
            "(SELECT COUNT(*) FROM favorites fv "
            "   WHERE fv.user_id = u.id) AS favorite_count, "
            "(SELECT COUNT(*) FROM notification_settings ns "
            "   WHERE ns.user_id = u.id AND ns.enabled = 1"
            ") AS notify_enabled "
            "FROM users u ORDER BY u.id",
        ).fetchall()
        result = []
        for row in rows:
            item = dict(row)
            item["is_admin"] = bool(item["is_admin"])
            item["notify_enabled"] = bool(item["notify_enabled"])
            result.append(item)
        return result
    finally:
        conn.close()


def set_user_admin(user_id: int, is_admin: bool) -> bool:
    """
    Set or revoke admin status for a user.

    Args:
        user_id: Target user ID.
        is_admin: True to grant, False to revoke.

    Returns:
        True if update succeeded.
    """
    conn = _get_connection()
    try:
        cur = conn.execute(
            "UPDATE users SET is_admin = ?, updated_at = ? WHERE id = ?",
            (int(is_admin), time.time(), user_id),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def delete_user(user_id: int) -> bool:
    """
    Delete a user and all cascaded data.

    Args:
        user_id: User ID to delete.

    Returns:
        True if deleted.
    """
    conn = _get_connection()
    try:
        cur = conn.execute("DELETE FROM users WHERE id = ?", (user_id,))
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def admin_reset_password(user_id: int, new_password: str) -> bool:
    """
    Admin password reset (no old password required).

    Args:
        user_id: Target user.
        new_password: New password.

    Returns:
        True on success.
    """
    salt = secrets.token_hex(16)
    pw_hash = _hash_password(new_password, salt)
    conn = _get_connection()
    try:
        cur = conn.execute(
            "UPDATE users SET password_hash = ?, salt = ?, updated_at = ? WHERE id = ?",
            (pw_hash, salt, time.time(), user_id),
        )
        if cur.rowcount > 0:
            conn.execute(
                "DELETE FROM access_tokens WHERE user_id = ?",
                (user_id,),
            )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def list_all_invite_codes() -> list[dict]:
    """
    List all invite codes with creator and consumer info.

    Returns:
        List of invite code dicts.
    """
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT ic.id, ic.code, ic.created_by, ic.used_by, "
            "ic.used_at, ic.created_at, "
            "uc.username AS created_by_name, "
            "uu.username AS used_by_name "
            "FROM invite_codes ic "
            "LEFT JOIN users uc ON ic.created_by = uc.id "
            "LEFT JOIN users uu ON ic.used_by = uu.id "
            "ORDER BY ic.created_at DESC",
        ).fetchall()
        return [dict(r) for r in rows]
    finally:
        conn.close()


def admin_create_invite_code() -> dict:
    """
    Create an invite code without a creator (admin-generated).

    Returns:
        Dict with id, code, created_at.
    """
    code = secrets.token_urlsafe(INVITE_CODE_BYTES)
    now = time.time()
    conn = _get_connection()
    try:
        cur = conn.execute(
            "INSERT INTO invite_codes "
            "(code, created_by, created_at) "
            "VALUES (?, NULL, ?)",
            (code, now),
        )
        conn.commit()
        return {"id": cur.lastrowid, "code": code, "created_at": now}
    finally:
        conn.close()


def delete_invite_code(code_id: int) -> bool:
    """
    Delete an invite code.

    Args:
        code_id: Invite code row ID.

    Returns:
        True if deleted.
    """
    conn = _get_connection()
    try:
        cur = conn.execute(
            "DELETE FROM invite_codes WHERE id = ? AND used_by IS NULL",
            (code_id,),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def get_auth_stats() -> dict:
    """
    Get comprehensive auth database statistics.

    Returns:
        Dict with user, folder, favorite, invite, notification counts.
    """
    conn = _get_connection()
    try:
        current_time = time.time()
        purge_expired_access_tokens(now=current_time, conn=conn)
        stats: dict = {}
        stats["total_users"] = conn.execute("SELECT COUNT(*) FROM users").fetchone()[0]
        stats["admin_count"] = conn.execute(
            "SELECT COUNT(*) FROM users WHERE is_admin = 1"
        ).fetchone()[0]
        stats["total_folders"] = conn.execute(
            "SELECT COUNT(*) FROM folders"
        ).fetchone()[0]
        stats["total_favorites"] = conn.execute(
            "SELECT COUNT(*) FROM favorites"
        ).fetchone()[0]
        stats["total_invite_codes"] = conn.execute(
            "SELECT COUNT(*) FROM invite_codes"
        ).fetchone()[0]
        stats["used_invite_codes"] = conn.execute(
            "SELECT COUNT(*) FROM invite_codes WHERE used_by IS NOT NULL"
        ).fetchone()[0]
        stats["unused_invite_codes"] = (
            stats["total_invite_codes"] - stats["used_invite_codes"]
        )
        stats["active_tokens"] = conn.execute(
            "SELECT COUNT(*) FROM access_tokens WHERE expires_at > ?",
            (current_time,),
        ).fetchone()[0]
        stats["notification_subscribers"] = conn.execute(
            "SELECT COUNT(*) FROM notification_settings WHERE enabled = 1"
        ).fetchone()[0]
        stats["scheduled_tasks"] = conn.execute(
            "SELECT COUNT(*) FROM scheduled_tasks"
        ).fetchone()[0]
        stats["active_announcements"] = conn.execute(
            "SELECT COUNT(*) FROM announcements WHERE enabled = 1"
        ).fetchone()[0]
        conn.commit()
        return stats
    finally:
        conn.close()


def _scheduled_task_from_row(row: sqlite3.Row) -> dict:
    """
    Convert a scheduled task row into a response dict.

    Args:
        row: SQLite row object.

    Returns:
        Scheduled task payload dict.
    """
    item = dict(row)
    item["enabled"] = bool(item["enabled"])
    return item


def list_scheduled_tasks() -> list[dict]:
    """
    List all scheduled tasks.

    Returns:
        Scheduled task payloads ordered by creation time.
    """
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT id, name, command, cron, enabled, "
            "last_run_at, last_status, created_at, updated_at "
            "FROM scheduled_tasks ORDER BY created_at DESC"
        ).fetchall()
        return [_scheduled_task_from_row(row) for row in rows]
    finally:
        conn.close()


def get_scheduled_task(task_id: int) -> dict | None:
    """
    Fetch one scheduled task by id.

    Args:
        task_id: Scheduled task identifier.

    Returns:
        Scheduled task payload or None.
    """
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, name, command, cron, enabled, "
            "last_run_at, last_status, created_at, updated_at "
            "FROM scheduled_tasks WHERE id = ?",
            (task_id,),
        ).fetchone()
        return _scheduled_task_from_row(row) if row else None
    finally:
        conn.close()


def create_scheduled_task(
    name: str,
    command: str,
    cron: str,
    enabled: bool = True,
) -> dict:
    """
    Create a scheduled task.

    Args:
        name: Display name.
        command: Shell command to run.
        cron: Five-field cron expression.
        enabled: Whether the task is active.

    Returns:
        Created scheduled task payload.
    """
    now = time.time()
    conn = _get_connection()
    try:
        cur = conn.execute(
            "INSERT INTO scheduled_tasks "
            "(name, command, cron, enabled, last_run_at, "
            "last_status, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, NULL, '', ?, ?)",
            (name, command, cron, int(enabled), now, now),
        )
        conn.commit()
        row = conn.execute(
            "SELECT id, name, command, cron, enabled, "
            "last_run_at, last_status, created_at, updated_at "
            "FROM scheduled_tasks WHERE id = ?",
            (cur.lastrowid,),
        ).fetchone()
        if row is None:
            raise RuntimeError("Scheduled task creation failed")
        return _scheduled_task_from_row(row)
    finally:
        conn.close()


def update_scheduled_task(
    task_id: int,
    *,
    name: str | None = None,
    command: str | None = None,
    cron: str | None = None,
    enabled: bool | None = None,
) -> dict | None:
    """
    Update one scheduled task.

    Args:
        task_id: Scheduled task identifier.
        name: Optional updated name.
        command: Optional updated command.
        cron: Optional updated cron expression.
        enabled: Optional updated enabled flag.

    Returns:
        Updated scheduled task payload or None.
    """
    current = get_scheduled_task(task_id)
    if current is None:
        return None

    now = time.time()
    conn = _get_connection()
    try:
        conn.execute(
            "UPDATE scheduled_tasks SET name = ?, command = ?, cron = ?, "
            "enabled = ?, updated_at = ? WHERE id = ?",
            (
                name if name is not None else current["name"],
                command if command is not None else current["command"],
                cron if cron is not None else current["cron"],
                int(enabled if enabled is not None else current["enabled"]),
                now,
                task_id,
            ),
        )
        conn.commit()
    finally:
        conn.close()
    return get_scheduled_task(task_id)


def delete_scheduled_task(task_id: int) -> bool:
    """
    Delete one scheduled task.

    Args:
        task_id: Scheduled task identifier.

    Returns:
        True if a task row was deleted.
    """
    conn = _get_connection()
    try:
        cur = conn.execute("DELETE FROM scheduled_tasks WHERE id = ?", (task_id,))
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def record_scheduled_task_run(task_id: int, status: str, ran_at: float) -> bool:
    """
    Store the result of one scheduled task run.

    Args:
        task_id: Scheduled task identifier.
        status: Run status string.
        ran_at: Execution timestamp.

    Returns:
        True if the task exists and was updated.
    """
    conn = _get_connection()
    try:
        cur = conn.execute(
            "UPDATE scheduled_tasks SET last_run_at = ?, last_status = ?, "
            "updated_at = ? WHERE id = ?",
            (ran_at, status, time.time(), task_id),
        )
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()


def _announcement_from_row(row: sqlite3.Row) -> dict:
    """
    Convert an announcement row into a response dict.

    Args:
        row: SQLite row object.

    Returns:
        Announcement payload dict.
    """
    item = dict(row)
    item["enabled"] = bool(item["enabled"])
    return item


def list_active_announcements() -> list[dict]:
    """
    List enabled announcements.

    Returns:
        Announcement payloads ordered by priority and recency.
    """
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT id, title, message, priority, enabled, created_at, updated_at "
            "FROM announcements WHERE enabled = 1 "
            "ORDER BY CASE priority "
            "WHEN 'high' THEN 0 "
            "WHEN 'normal' THEN 1 "
            "ELSE 2 END, created_at DESC"
        ).fetchall()
        return [_announcement_from_row(row) for row in rows]
    finally:
        conn.close()


def list_all_announcements() -> list[dict]:
    """
    List all announcements for admin management.

    Returns:
        Announcement payloads ordered by creation time.
    """
    conn = _get_connection()
    try:
        rows = conn.execute(
            "SELECT id, title, message, priority, enabled, created_at, updated_at "
            "FROM announcements ORDER BY created_at DESC"
        ).fetchall()
        return [_announcement_from_row(row) for row in rows]
    finally:
        conn.close()


def get_announcement(announcement_id: int) -> dict | None:
    """
    Fetch one announcement by id.

    Args:
        announcement_id: Announcement identifier.

    Returns:
        Announcement payload or None.
    """
    conn = _get_connection()
    try:
        row = conn.execute(
            "SELECT id, title, message, priority, enabled, created_at, updated_at "
            "FROM announcements WHERE id = ?",
            (announcement_id,),
        ).fetchone()
        return _announcement_from_row(row) if row else None
    finally:
        conn.close()


def create_announcement(
    title: str,
    message: str,
    priority: str = "normal",
    enabled: bool = True,
) -> dict:
    """
    Create an announcement.

    Args:
        title: Announcement title.
        message: Announcement body.
        priority: Priority label.
        enabled: Whether the announcement is visible.

    Returns:
        Created announcement payload.
    """
    now = time.time()
    conn = _get_connection()
    try:
        cur = conn.execute(
            "INSERT INTO announcements "
            "(title, message, priority, enabled, created_at, updated_at) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (title, message, priority, int(enabled), now, now),
        )
        conn.commit()
        row = conn.execute(
            "SELECT id, title, message, priority, enabled, created_at, updated_at "
            "FROM announcements WHERE id = ?",
            (cur.lastrowid,),
        ).fetchone()
        if row is None:
            raise RuntimeError("Announcement creation failed")
        return _announcement_from_row(row)
    finally:
        conn.close()


def update_announcement(
    announcement_id: int,
    *,
    title: str | None = None,
    message: str | None = None,
    priority: str | None = None,
    enabled: bool | None = None,
) -> dict | None:
    """
    Update one announcement.

    Args:
        announcement_id: Announcement identifier.
        title: Optional updated title.
        message: Optional updated message.
        priority: Optional updated priority.
        enabled: Optional updated enabled flag.

    Returns:
        Updated announcement payload or None.
    """
    current = get_announcement(announcement_id)
    if current is None:
        return None

    now = time.time()
    conn = _get_connection()
    try:
        conn.execute(
            "UPDATE announcements SET title = ?, message = ?, priority = ?, "
            "enabled = ?, updated_at = ? WHERE id = ?",
            (
                title if title is not None else current["title"],
                message if message is not None else current["message"],
                priority if priority is not None else current["priority"],
                int(enabled if enabled is not None else current["enabled"]),
                now,
                announcement_id,
            ),
        )
        conn.commit()
    finally:
        conn.close()
    return get_announcement(announcement_id)


def delete_announcement(announcement_id: int) -> bool:
    """
    Delete one announcement.

    Args:
        announcement_id: Announcement identifier.

    Returns:
        True if an announcement row was deleted.
    """
    conn = _get_connection()
    try:
        cur = conn.execute("DELETE FROM announcements WHERE id = ?", (announcement_id,))
        conn.commit()
        return cur.rowcount > 0
    finally:
        conn.close()
