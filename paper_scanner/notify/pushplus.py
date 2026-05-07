"""PushPlus delivery client."""

from __future__ import annotations

import time
from typing import Any

import httpx

from paper_scanner.notify.models import PUSHPLUS_ENDPOINT
from paper_scanner.shared.converters import to_int


class PushPlusClient:
    """
    PushPlus delivery client with retries.

    Args:
        timeout_seconds: Request timeout.
        retries: Retry attempts for transient failures.
    """

    def __init__(self, timeout_seconds: int, retries: int) -> None:
        """
        Initialize PushPlus client.

        Args:
            timeout_seconds: Request timeout.
            retries: Retry attempts.

        Returns:
            None.
        """
        self.retries = max(0, retries)
        self.client = httpx.Client(timeout=timeout_seconds)

    def close(self) -> None:
        """
        Close HTTP resources.

        Args:
            None.

        Returns:
            None.
        """
        self.client.close()

    def send(
        self,
        token: str,
        title: str,
        content: str,
        channel: str,
        template: str,
        topic: str | None,
        option: str | None,
        to: str | None,
    ) -> str:
        """
        Send PushPlus message and return message id.

        Args:
            token: PushPlus token.
            title: Message title.
            content: Message content.
            channel: PushPlus channel.
            template: PushPlus template.
            topic: Optional topic.
            option: Optional channel option value.
            to: Optional recipient value.

        Returns:
            PushPlus message id.
        """
        payload: dict[str, Any] = {
            "token": token,
            "title": title,
            "content": content,
            "channel": channel,
            "template": template,
        }
        if to:
            payload["to"] = to
        if topic:
            payload["topic"] = topic
        if option:
            payload["option"] = option

        last_error: Exception | None = None
        for attempt in range(self.retries + 1):
            try:
                response = self.client.post(PUSHPLUS_ENDPOINT, json=payload)
                if (
                    response.status_code in {429, 500, 502, 503, 504}
                    and attempt < self.retries
                ):
                    time.sleep(2**attempt)
                    continue
                response.raise_for_status()
                result = response.json()
                if not isinstance(result, dict):
                    raise ValueError("PushPlus response is not a JSON object")
                code = to_int(result.get("code"))
                if code != 200:
                    message = str(result.get("msg") or "Unknown PushPlus error")
                    raise RuntimeError(f"PushPlus failed with code {code}: {message}")
                return str(result.get("data") or "")
            except Exception as error:
                last_error = error
                if attempt < self.retries:
                    time.sleep(2**attempt)
                    continue
                break
        raise RuntimeError(f"PushPlus request failed: {last_error}")
