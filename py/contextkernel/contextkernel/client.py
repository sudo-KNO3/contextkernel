"""HTTP client for the ContextKernel server."""
from __future__ import annotations

from typing import Any, Optional

import httpx

from .types import (
    BundleItem,
    ContextBundle,
    QueueEntry,
    QueueResponse,
    VaultStats,
)

DEFAULT_BASE_URL = "http://127.0.0.1:9292"


class ContextKernel:
    """Thin client around the ContextKernel HTTP API.

    The server is normally started with ``ctxk serve`` and binds to
    127.0.0.1:9292 by default. All requests are JSON.
    """

    def __init__(
        self,
        base_url: str = DEFAULT_BASE_URL,
        *,
        timeout: float = 10.0,
        client: Optional[httpx.Client] = None,
    ):
        self._base = base_url.rstrip("/")
        self._client = client or httpx.Client(timeout=timeout)

    # ─── context ────────────────────────────────────────────────────────────

    def query(
        self,
        task: str,
        *,
        scope: Optional[str] = None,
        scope_path: Optional[str] = None,
        knowledge_types: Optional[list[str]] = None,
        domains: Optional[list[str]] = None,
        tags_any: Optional[list[str]] = None,
        include_stale: bool = False,
        max_items: int = 12,
        include_conflicts: bool = True,
    ) -> ContextBundle:
        body: dict[str, Any] = {
            "task": task,
            "include_stale": include_stale,
            "max_items": max_items,
            "include_conflicts": include_conflicts,
        }
        if scope is not None:
            body["scope"] = scope
        if scope_path is not None:
            body["scope_path"] = scope_path
        if knowledge_types is not None:
            body["knowledge_types"] = knowledge_types
        if domains is not None:
            body["domains"] = domains
        if tags_any is not None:
            body["tags_any"] = tags_any
        data = self._post("/context/query", body)
        return ContextBundle.model_validate(data)

    # ─── knowledge CRUD ─────────────────────────────────────────────────────

    def get(self, item_id: str) -> BundleItem:
        data = self._get(f"/knowledge/{item_id}")
        # The single-item endpoint returns a full KnowledgeItem, which is a
        # superset of BundleItem fields; reuse BundleItem for the typed view.
        return BundleItem.model_validate({**data, "score": 1.0,
                                          "score_breakdown": {}})

    def list(self, **params: Any) -> list[BundleItem]:
        data = self._get("/knowledge", params=params)
        return [BundleItem.model_validate({**i, "score": 1.0,
                                           "score_breakdown": {}}) for i in data]

    # ─── propose / review ───────────────────────────────────────────────────

    def propose(
        self,
        *,
        knowledge_type: str,
        scope: str,
        title: str,
        body_html: str,
        confidence: float = 0.7,
        source_type: str = "agent",
        tags: Optional[list[str]] = None,
        domain: Optional[str] = None,
        stability: str = "medium-term",
        proposed_by: str = "agent:python",
        rationale: Optional[str] = None,
    ) -> QueueResponse:
        item = {
            "knowledge_type": knowledge_type,
            "scope": scope,
            "title": title,
            "body_html": body_html,
            "confidence": confidence,
            "source_type": source_type,
            "tags": tags or [],
            "stability": stability,
        }
        if domain is not None:
            item["domain"] = domain
        body = {
            "proposed_by": proposed_by,
            "rationale": rationale,
            "item": item,
        }
        data = self._post("/knowledge/propose", body)
        return QueueResponse.model_validate(data)

    def propose_update(
        self,
        item_id: str,
        *,
        patch: dict[str, Any],
        proposed_by: str = "agent:python",
        rationale: Optional[str] = None,
    ) -> QueueResponse:
        body = {"proposed_by": proposed_by, "rationale": rationale, "patch": patch}
        data = self._patch(f"/knowledge/{item_id}/propose-update", body)
        return QueueResponse.model_validate(data)

    def queue(self, status: Optional[str] = "pending") -> list[QueueEntry]:
        params = {"status": status} if status else {}
        data = self._get("/review/queue", params=params)
        return [QueueEntry.model_validate(e) for e in data]

    # ─── admin ──────────────────────────────────────────────────────────────

    def stats(self) -> VaultStats:
        data = self._get("/vault/stats")
        return VaultStats.model_validate(data)

    def reindex(self, path: Optional[str] = None) -> dict[str, Any]:
        body = {"path": path} if path else {}
        return self._post("/vault/reindex", body)

    def health(self) -> bool:
        try:
            r = self._client.get(f"{self._base}/health")
            r.raise_for_status()
            return r.text.strip() == "ok"
        except httpx.HTTPError:
            return False

    # ─── internals ──────────────────────────────────────────────────────────

    def _get(self, path: str, params: Optional[dict[str, Any]] = None) -> Any:
        r = self._client.get(f"{self._base}{path}", params=params)
        r.raise_for_status()
        return r.json()

    def _post(self, path: str, body: dict[str, Any]) -> Any:
        r = self._client.post(f"{self._base}{path}", json=body)
        r.raise_for_status()
        return r.json()

    def _patch(self, path: str, body: dict[str, Any]) -> Any:
        r = self._client.patch(f"{self._base}{path}", json=body)
        r.raise_for_status()
        return r.json()

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> "ContextKernel":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()
