"""Shadow contracts for Rust auth database business endpoints."""

from __future__ import annotations

import unittest
from typing import Any

from fastapi import FastAPI
from fastapi.testclient import TestClient

import paper_scanner.api.auth_db as auth_db
from paper_scanner.api.routes import register_routes
from paper_scanner.api.routes import tracking as tracking_routes

from .test_auth_api_contracts import cookie_pair, first_set_cookie, request_json
from .test_public_api_contracts import (
    available_port,
    start_rust_api,
    stop_process,
    temporary_auth_database,
)


def login_cookie(base_url: str, username: str, password: str) -> str:
    """
    Login through the Rust API and return the browser session cookie pair.

    Args:
        base_url: Rust API base URL.
        username: Username.
        password: Password.

    Returns:
        Cookie header value.
    """
    status, _payload, headers = request_json(
        "POST",
        f"{base_url}/api/auth/login",
        {"username": username, "password": password},
    )
    if status != 200:
        raise AssertionError(f"Login failed with status {status}")
    return cookie_pair(first_set_cookie(headers))


def bearer_headers(token: dict[str, Any]) -> dict[str, str]:
    """
    Build Python TestClient authorization headers for an access token.

    Args:
        token: Access token payload from Python auth_db.

    Returns:
        Authorization headers.
    """
    return {"Authorization": f"Bearer {token['token']}"}


class AuthBusinessApiContractTest(unittest.TestCase):
    """Compare migrated Rust business routes with Python on one auth DB."""

    def test_favorites_and_tracking_routes_share_python_database_contract(self) -> None:
        """
        Verify folder, favorite, bulk, check, and tracking folder behavior.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            owner = auth_db.register_with_invite("owner", "secret123")
            token = auth_db.create_access_token(int(owner["id"]), name="api")
            app = FastAPI()
            register_routes(app)
            client = TestClient(app)
            port = available_port()
            process = start_rust_api(project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "owner", "secret123")
                python_folders = client.get(
                    "/api/favorites/folders",
                    headers=bearer_headers(token),
                )
                rust_status, rust_folders, _headers = request_json(
                    "GET",
                    f"{base_url}/api/favorites/folders",
                    headers={"Cookie": cookie},
                )

                self.assertEqual(rust_status, python_folders.status_code)
                self.assertEqual(rust_folders, python_folders.json())
                default_folder_id = rust_folders[0]["id"]

                create_status, created_folder, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/folders",
                    {"name": "Research", "is_tracking": False},
                    headers={"Cookie": cookie},
                )
                duplicate_status, duplicate_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/folders",
                    {"name": "Research", "is_tracking": False},
                    headers={"Cookie": cookie},
                )
                add_status, added, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/folders/{created_folder['id']}/articles",
                    {"article_id": 101, "db_name": "demo.sqlite", "note": "keep"},
                    headers={"Cookie": cookie},
                )
                bulk_status, bulk_added, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/folders/{created_folder['id']}/articles/bulk",
                    {
                        "articles": [
                            {"article_id": 202, "db_name": "demo.sqlite", "note": ""},
                            {"article_id": 303, "db_name": "demo.sqlite", "note": ""},
                        ]
                    },
                    headers={"Cookie": cookie},
                )
                batch_status, batch_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/check/batch",
                    {"article_ids": [101, 202, 202, 0], "db_name": "demo.sqlite"},
                    headers={"Cookie": cookie},
                )
                tracking_status, tracking_payload, _headers = request_json(
                    "PUT",
                    f"{base_url}/api/favorites/tracking",
                    {"folder_id": created_folder["id"]},
                    headers={"Cookie": cookie},
                )
                move_status, moved, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/folders/{created_folder['id']}/articles/bulk-move",
                    {
                        "target_folder_id": default_folder_id,
                        "articles": [{"article_id": 202, "db_name": "demo.sqlite"}],
                    },
                    headers={"Cookie": cookie},
                )
                remove_status, removed, _headers = request_json(
                    "POST",
                    f"{base_url}/api/favorites/folders/{default_folder_id}/articles/bulk-remove",
                    {"articles": [{"article_id": 202, "db_name": "demo.sqlite"}]},
                    headers={"Cookie": cookie},
                )

                python_check = client.get(
                    "/api/favorites/check?article_id=101&db_name=demo.sqlite",
                    headers=bearer_headers(token),
                )
                rust_check_status, rust_check, _headers = request_json(
                    "GET",
                    f"{base_url}/api/favorites/check?article_id=101&db_name=demo.sqlite",
                    headers={"Cookie": cookie},
                )
                python_tracking = client.get(
                    "/api/favorites/tracking",
                    headers=bearer_headers(token),
                )

                self.assertEqual(create_status, 200)
                self.assertEqual(created_folder["name"], "Research")
                self.assertEqual(duplicate_status, 409)
                self.assertEqual(
                    duplicate_payload, {"detail": "Folder name already exists"}
                )
                self.assertEqual(add_status, 200)
                self.assertEqual(added["article_id"], "101")
                self.assertEqual(bulk_status, 200)
                self.assertEqual(bulk_added, {"added": 2})
                self.assertEqual(batch_status, 200)
                self.assertEqual(
                    [item["article_id"] for item in batch_payload], ["101", "202"]
                )
                self.assertEqual(tracking_status, 200)
                self.assertEqual(tracking_payload, {"ok": True})
                self.assertEqual(move_status, 200)
                self.assertEqual(moved, {"count": 1})
                self.assertEqual(remove_status, 200)
                self.assertEqual(removed, {"count": 1})
                self.assertEqual(rust_check_status, python_check.status_code)
                self.assertEqual(rust_check, python_check.json())
                self.assertEqual(
                    python_tracking.json(),
                    {
                        "folder_id": created_folder["id"],
                        "folder_name": "Research",
                    },
                )
            finally:
                stop_process(process)
                client.close()

    def test_admin_settings_and_announcement_routes_match_python_visibility(
        self,
    ) -> None:
        """
        Verify admin, settings, scheduled task, and announcement mutations.

        Returns:
            None.
        """
        with temporary_auth_database() as project_root:
            previous_tracking_root = tracking_routes.PROJECT_ROOT
            tracking_routes.PROJECT_ROOT = project_root
            owner = auth_db.register_with_invite("owner", "secret123")
            other = auth_db.create_user("reader", "oldsecret")
            token = auth_db.create_access_token(int(owner["id"]), name="api")
            app = FastAPI()
            register_routes(app)
            client = TestClient(app)
            port = available_port()
            process = start_rust_api(project_root, port)
            base_url = f"http://127.0.0.1:{port}"
            try:
                cookie = login_cookie(base_url, "owner", "secret123")
                rust_users_status, rust_users, _headers = request_json(
                    "GET",
                    f"{base_url}/api/admin/users",
                    headers={"Cookie": cookie},
                )
                python_users = client.get(
                    "/api/admin/users",
                    headers=bearer_headers(token),
                )

                notification_status, notification_payload, _headers = request_json(
                    "PUT",
                    f"{base_url}/api/tracking/notification-settings",
                    {
                        "keywords": [" ai ", ""],
                        "directions": [" systems "],
                        "selected_databases": [],
                        "delivery_method": "folder",
                        "pushplus_token": "",
                        "pushplus_template": "",
                        "pushplus_topic": " topic ",
                        "pushplus_channel": "wechat",
                        "sync_to_tracking_folder": False,
                        "ai_base_url": " https://api.example.test ",
                        "ai_api_key": " key ",
                        "ai_model": " model ",
                        "ai_system_prompt": " prompt ",
                        "ai_backup_base_url": "",
                        "ai_backup_api_key": "",
                        "ai_backup_model": "",
                        "ai_backup_system_prompt": "",
                        "ai_retry_attempts": 4,
                        "enabled": True,
                    },
                    headers={"Cookie": cookie},
                )
                python_notification = client.get(
                    "/api/tracking/notification-settings",
                    headers=bearer_headers(token),
                )
                rust_tracking_status, rust_tracking, _headers = request_json(
                    "GET",
                    f"{base_url}/api/tracking/status",
                    headers={"Cookie": cookie},
                )
                python_tracking = client.get(
                    "/api/tracking/status",
                    headers=bearer_headers(token),
                )

                runtime_status, runtime_payload, _headers = request_json(
                    "PUT",
                    f"{base_url}/api/admin/runtime-settings",
                    {"values": {"crossref_mailto_pool": "admin@example.test"}},
                    headers={"Cookie": cookie},
                )
                python_runtime = client.get(
                    "/api/admin/runtime-settings",
                    headers=bearer_headers(token),
                )

                task_status, task_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/admin/scheduled-tasks",
                    {
                        "name": "Index",
                        "command": "uv run index",
                        "cron": "* * * * *",
                        "enabled": True,
                    },
                    headers={"Cookie": cookie},
                )
                task_update_status, task_updated, _headers = request_json(
                    "PUT",
                    f"{base_url}/api/admin/scheduled-tasks/{task_payload['id']}",
                    {"enabled": False},
                    headers={"Cookie": cookie},
                )
                task_delete_status, task_deleted, _headers = request_json(
                    "DELETE",
                    f"{base_url}/api/admin/scheduled-tasks/{task_payload['id']}",
                    headers={"Cookie": cookie},
                )

                announcement_status, announcement, _headers = request_json(
                    "POST",
                    f"{base_url}/api/admin/announcements",
                    {
                        "title": "  Upgrade ",
                        "message": "  Ready ",
                        "priority": "HIGH",
                        "enabled": True,
                    },
                    headers={"Cookie": cookie},
                )
                announcement_update_status, announcement_updated, _headers = (
                    request_json(
                        "PUT",
                        f"{base_url}/api/admin/announcements/{announcement['id']}",
                        {"enabled": False, "priority": "low"},
                        headers={"Cookie": cookie},
                    )
                )
                announcement_delete_status, announcement_deleted, _headers = (
                    request_json(
                        "DELETE",
                        f"{base_url}/api/admin/announcements/{announcement['id']}",
                        headers={"Cookie": cookie},
                    )
                )

                invite_status, invite_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/admin/invite-codes",
                    headers={"Cookie": cookie},
                )
                invite_delete_status, invite_deleted, _headers = request_json(
                    "DELETE",
                    f"{base_url}/api/admin/invite-codes/{invite_payload['id']}",
                    headers={"Cookie": cookie},
                )

                reset_status, reset_payload, _headers = request_json(
                    "POST",
                    f"{base_url}/api/admin/users/{other['id']}/reset-password",
                    {"new_password": "newsecret"},
                    headers={"Cookie": cookie},
                )

                self.assertEqual(rust_users_status, python_users.status_code)
                self.assertEqual(rust_users, python_users.json())
                self.assertEqual(notification_status, 200)
                self.assertEqual(notification_payload, python_notification.json())
                self.assertEqual(notification_payload["keywords"], ["ai"])
                self.assertEqual(notification_payload["directions"], ["systems"])
                self.assertEqual(notification_payload["pushplus_template"], "markdown")
                self.assertEqual(rust_tracking_status, python_tracking.status_code)
                self.assertEqual(rust_tracking, python_tracking.json())
                self.assertTrue(rust_tracking["notification_configured"])
                self.assertEqual(runtime_status, 200)
                self.assertEqual(runtime_payload, python_runtime.json())
                self.assertIn(
                    {
                        "field": "crossref_mailto_pool",
                        "key": "CROSSREF_MAILTO_POOL",
                        "label": "Crossref mailto pool",
                        "description": (
                            "Comma- or semicolon-separated Crossref contact emails."
                        ),
                        "input_type": "text",
                        "is_secret": False,
                        "value": "admin@example.test",
                        "source": "database",
                        "updated_at": runtime_payload[2]["updated_at"],
                    },
                    runtime_payload,
                )
                self.assertEqual(task_status, 200)
                self.assertEqual(task_payload["last_status"], "")
                self.assertEqual(task_update_status, 200)
                self.assertFalse(task_updated["enabled"])
                self.assertEqual(task_delete_status, 200)
                self.assertEqual(task_deleted, {"ok": True})
                self.assertEqual(announcement_status, 200)
                self.assertEqual(announcement["priority"], "high")
                self.assertEqual(announcement["title"], "Upgrade")
                self.assertEqual(announcement_update_status, 200)
                self.assertFalse(announcement_updated["enabled"])
                self.assertEqual(announcement_updated["priority"], "low")
                self.assertEqual(announcement_delete_status, 200)
                self.assertEqual(announcement_deleted, {"ok": True})
                self.assertEqual(invite_status, 200)
                self.assertEqual(sorted(invite_payload), ["code", "created_at", "id"])
                self.assertEqual(invite_delete_status, 200)
                self.assertEqual(invite_deleted, {"ok": True})
                self.assertEqual(reset_status, 200)
                self.assertEqual(reset_payload, {"ok": True})
                self.assertEqual(
                    auth_db.verify_user("reader", "newsecret")["id"], other["id"]
                )
                delete_status, delete_payload, _headers = request_json(
                    "DELETE",
                    f"{base_url}/api/admin/users/{other['id']}",
                    headers={"Cookie": cookie},
                )
                self.assertEqual(delete_status, 200)
                self.assertEqual(delete_payload, {"ok": True})
                self.assertIsNone(auth_db.get_user_by_id(int(other["id"])))
            finally:
                tracking_routes.PROJECT_ROOT = previous_tracking_root
                stop_process(process)
                client.close()


if __name__ == "__main__":
    unittest.main()
