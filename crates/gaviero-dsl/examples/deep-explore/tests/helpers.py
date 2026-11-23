"""Builders for the deep-explore gate tests: C1-shaped run artefacts."""

from __future__ import annotations

import contextlib
import io
import json
import sys
from pathlib import Path
from typing import Any, Iterable, Optional

TOOLS_DIR = Path(__file__).resolve().parents[1]
if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))


def question(qid: str, **overrides: Any) -> dict:
    """An open, non-core question with priority 8 (enough to keep S1 from firing)."""
    q = {
        "id": qid,
        "question": f"What explains {qid}?",
        "kind": "mechanism",
        "operator": "seed",
        "parent": "",
        "depth": 0,
        "status": "open",
        "scores": {"relevance": 2, "gain": 2, "novelty": 2, "tractability": 2},
        "priority": 8,
        "visits": 0,
        "supported_claims": [],
        "why": f"evidence hook for {qid}",
    }
    q.update(overrides)
    return q


def frontier(n: int, questions: Iterable[dict], created_total: Optional[int] = None, leading_claims: Iterable[str] = ()) -> dict:
    qs = list(questions)

    def count(status: str) -> int:
        return sum(1 for q in qs if q.get("status") == status)

    return {
        "pass": n,
        "collection": "explore-test",
        "questions": qs,
        "leading_claims": list(leading_claims),
        "metrics": {
            "open": count("open") + count("active"),
            "closed": count("closed"),
            "dead": count("dead"),
            "parked": count("parked"),
            "created_total": len(qs) if created_total is None else created_total,
        },
    }


def claim(text: str, work_id: str = "W1", title: str = "A study of things", year: int = 2020) -> dict:
    return {
        "text": text,
        "work_id": work_id,
        "title": title,
        "year": year,
        "locator": "results",
        "quote": "",
        "evidence": "fulltext",
        "strength": "single_study",
        "confidence": "high",
        "channel": "academic",
    }


def child(text: str, kind: str = "mechanism", why: str = "a dispute") -> dict:
    return {"question": text, "kind": kind, "why": why}


def finding(aid: str, claims: Iterable[dict], children: Iterable[dict] = (), outcome: str = "answered", fidelity: str = "exact") -> dict:
    return {
        "assignment_id": aid,
        "question": f"question of {aid}",
        "outcome": outcome,
        "fidelity": fidelity,
        "fidelity_note": "",
        "analysis_run": "searched and read",
        "answer": "an answer",
        "claims": list(claims),
        "counter_evidence": [],
        "children": list(children),
        "dead_ends": [],
        "search_log": [],
    }


def work(
    openalex: str = "W1",
    title: str = "A study of things",
    doi: str = "",
    arxiv: str = "",
    year: int = 2020,
    venue: str = "Journal",
    authors: Iterable[str] = ("Ann Author",),
) -> dict:
    return {
        "openalex": openalex,
        "doi": doi,
        "arxiv": arxiv,
        "title": title,
        "year": year,
        "venue": venue,
        "authors": list(authors),
    }


def audit_claim(
    finding_id: str,
    index: int,
    verdict: str,
    work: Optional[dict] = None,
    retracted: bool = False,
    resolved: bool = True,
) -> dict:
    """An audit verdict; `work=None` means the default resolved work (W1)."""
    return {
        "finding": finding_id,
        "index": index,
        "verdict": verdict,
        "reason": "",
        "passage": "the passage",
        "checked_via": "fulltext",
        "retracted": retracted,
        "resolved": resolved,
        "sampled": False,
        "work": globals()["work"]() if work is None else work,
    }


def audit(
    n: int,
    claims: Iterable[dict],
    fidelity: Iterable[tuple] = (),
    novel_children: Optional[int] = 0,
    metrics: Optional[dict] = None,
) -> dict:
    """An audit whose metrics match its claims; `novel_children=None` omits the key."""
    cs = list(claims)
    m = {
        "claims": len(cs),
        "checked": len(cs),
        "supported": 0,
        "partial": 0,
        "not_supported": 0,
        "cannot_verify": 0,
        "unresolvable": sum(1 for c in cs if c.get("resolved") is False),
        "retracted": sum(1 for c in cs if c.get("retracted") is True),
        "new_children": 0,
    }
    for c in cs:
        if c.get("verdict") in m:
            m[c["verdict"]] += 1
    if novel_children is not None:
        m["novel_children"] = novel_children
    if metrics:
        m.update(metrics)
    return {
        "pass": n,
        "claims": cs,
        "fidelity": [{"finding": f, "verdict": v, "note": ""} for f, v in fidelity],
        "contradictions": [],
        "metrics": m,
        "saturation": "",
    }


def write_json(out_dir: Path, name: str, data: Any) -> Path:
    path = Path(out_dir) / name
    path.write_text(json.dumps(data, indent=1, ensure_ascii=False), encoding="utf-8")
    return path


def read_json(out_dir: Path, name: str) -> Any:
    return json.loads((Path(out_dir) / name).read_text(encoding="utf-8"))


def assignment(aid: str) -> dict:
    return {
        "id": aid,
        "question": f"question of {aid}",
        "lens": "exploit",
        "kind": "mechanism",
        "why": "hook",
        "start_from": "W1",
        "avoid": "",
        "done_when": "answered",
    }


def write_pass(
    out_dir: Path,
    n: int,
    frontier: Optional[dict] = None,
    assignments: Optional[Iterable] = None,
    findings: Optional[Iterable[dict]] = None,
    audit: Optional[dict] = None,
) -> None:
    """Write pass `n`'s artefacts; assignments may be ids or full items."""
    out = Path(out_dir)
    if frontier is not None:
        write_json(out, f"frontier-v{n}.json", frontier)
    if assignments is not None:
        items = [assignment(a) if isinstance(a, str) else a for a in assignments]
        write_json(out, f"assignments-v{n}.json", items)
    for f in findings or ():
        write_json(out, f"{f['assignment_id']}-finding-v{n}.json", f)
    if audit is not None:
        write_json(out, f"audit-v{n}.json", audit)


def run_main(module: Any, argv: list) -> tuple[int, str]:
    """Run `module.main(argv)` in-process; return (exit code, stdout)."""
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        code = module.main([str(a) for a in argv])
    return code, buffer.getvalue()
