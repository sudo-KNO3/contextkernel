"""Typed responses returned by the ContextKernel HTTP API."""
from __future__ import annotations

from typing import Optional

from pydantic import BaseModel, ConfigDict, Field


class RelationView(BaseModel):
    model_config = ConfigDict(extra="ignore")
    rel: str
    target: str


class ScoreBreakdown(BaseModel):
    model_config = ConfigDict(extra="ignore")
    fts: float = 0.0
    lexical: float = 0.0
    scope: float = 0.0
    recency: float = 0.0
    confidence: float = 0.0
    source: float = 0.0


class BundleItem(BaseModel):
    model_config = ConfigDict(extra="ignore")
    id: str
    score: float
    knowledge_type: str
    scope: str
    title: str
    body_html: str
    body_text: str
    confidence: float
    source_type: str
    status: str
    stability: str
    created: str
    modified: str
    valid_until: Optional[str] = None
    domain: Optional[str] = None
    tags: list[str] = Field(default_factory=list)
    relations: list[RelationView] = Field(default_factory=list)
    score_breakdown: ScoreBreakdown = Field(default_factory=ScoreBreakdown)


class Conflict(BaseModel):
    model_config = ConfigDict(extra="ignore")
    id: str
    claim_key: str
    scope: str
    item_ids: list[str]


class ContextBundle(BaseModel):
    model_config = ConfigDict(extra="ignore")
    query_id: str
    items: list[BundleItem] = Field(default_factory=list)
    conflicts: list[Conflict] = Field(default_factory=list)
    total_candidates: int = 0
    stale_excluded: int = 0


class QueueResponse(BaseModel):
    model_config = ConfigDict(extra="ignore")
    queue_id: str
    status: str
    target_id: Optional[str] = None


class QueueEntry(BaseModel):
    model_config = ConfigDict(extra="ignore")
    id: str
    kind: str
    target_id: Optional[str] = None
    proposed_by: str
    proposed_at: str
    status: str
    payload_json: str
    rationale: Optional[str] = None


class VaultStats(BaseModel):
    model_config = ConfigDict(extra="ignore")
    total: int
    by_scope: list[tuple[str, int]] = Field(default_factory=list)
    by_type: list[tuple[str, int]] = Field(default_factory=list)
