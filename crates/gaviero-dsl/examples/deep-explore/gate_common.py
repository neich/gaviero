"""Shared definitions for the deep-explore gate scripts.

Implements contract C2 of plans/deep-explore-gates/deep-explore-gates-v1.md:
claim keys, title comparison, canonical work keys, versioned artefact
discovery, JSON I/O and the claim ledger (`build_ledger`) that both
`pass_gate.py` and `report_gate.py` rebuild from the run's findings and
audits.

Standard library only; Python 3.10 or newer.
"""

from __future__ import annotations

import difflib
import json
import os
import re
import tempfile
from pathlib import Path
from typing import Any, Optional

TITLE_MISMATCH_THRESHOLD = 0.70

VERDICTS = ("supported", "partial", "not_supported", "cannot_verify")
WORK_TEXT_FIELDS = ("openalex", "doi", "arxiv", "title", "venue")


class GateInputError(Exception):
    """A required input artefact is missing or cannot be read."""


# ── small coercions ──────────────────────────────────────────────────────────


def as_str(value: Any) -> str:
    """`value` as a string; `None` becomes the empty string."""
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    return str(value)


def as_int(value: Any) -> int:
    """`value` as an int; anything that is not a number becomes 0."""
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    if isinstance(value, str) and re.fullmatch(r"\s*-?\d+\s*", value):
        return int(value)
    return 0


def as_list(value: Any) -> list:
    """`value` if it is a list, else an empty list."""
    return value if isinstance(value, list) else []


def as_dict(value: Any) -> dict:
    """`value` if it is a dict, else an empty dict."""
    return value if isinstance(value, dict) else {}


# ── JSON I/O ─────────────────────────────────────────────────────────────────


def load_json(path: Path) -> Any:
    """Parse a JSON file; raise `GateInputError("<name>: <reason>")` on failure."""
    path = Path(path)
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        raise GateInputError(f"{path.name}: file not found") from None
    except (OSError, UnicodeDecodeError) as exc:
        raise GateInputError(f"{path.name}: cannot read ({exc})") from None
    try:
        return json.loads(text.lstrip("﻿"))
    except json.JSONDecodeError as exc:
        raise GateInputError(f"{path.name}: invalid JSON ({exc})") from None


def write_text(path: Path, text: str) -> None:
    """Write UTF-8 text atomically: a temp file in the same folder, then replace."""
    path = Path(path)
    fd, tmp = tempfile.mkstemp(prefix="." + path.name + ".", suffix=".tmp", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def write_json(path: Path, data: Any) -> None:
    """Write `data` as indented UTF-8 JSON, atomically."""
    write_text(path, json.dumps(data, indent=1, ensure_ascii=False) + "\n")


# ── keys and titles ──────────────────────────────────────────────────────────


def claim_key(assignment_id: str, index: int) -> str:
    """`<assignment id>#<claim index>`, e.g. `q7-p2#0`."""
    return f"{assignment_id}#{index}"


def question_id_of(aid: str) -> str:
    """The question id of an assignment id: `q7-p2` -> `q7`."""
    match = re.fullmatch(r"(.+)-p\d+", aid)
    return match.group(1) if match else aid


def normalize_title(s: Any) -> str:
    """Lower-case, every run of non-alphanumerics replaced by one space."""
    return re.sub(r"[^0-9a-z]+", " ", as_str(s).lower()).strip()


def title_ratio(a: Any, b: Any) -> Optional[float]:
    """Similarity of two titles in [0, 1]; `None` when either is empty."""
    na, nb = normalize_title(a), normalize_title(b)
    if not na or not nb:
        return None
    return difflib.SequenceMatcher(None, na, nb).ratio()


def _strip_prefix(value: str, prefix: str) -> str:
    if value.lower().startswith(prefix):
        return value[len(prefix):]
    return value


def clean_openalex(value: Any) -> str:
    return _strip_prefix(as_str(value).strip(), "https://openalex.org/")


def clean_doi(value: Any) -> str:
    return _strip_prefix(as_str(value).strip(), "https://doi.org/")


def work_key(work: Any) -> Optional[str]:
    """Canonical key of a resolved work: OpenAlex id, else DOI, else arXiv, else title."""
    if not isinstance(work, dict):
        return None
    openalex = clean_openalex(work.get("openalex"))
    if re.fullmatch(r"W\d+", openalex):
        return openalex
    doi = clean_doi(work.get("doi")).lower()
    if doi:
        return "doi:" + doi
    arxiv = as_str(work.get("arxiv")).strip()
    if arxiv:
        return "arxiv:" + arxiv
    title = normalize_title(work.get("title"))
    if title:
        return "title:" + title
    return None


# ── versioned artefacts ──────────────────────────────────────────────────────


def versioned_files(out_dir: Path, stem: str) -> dict[int, Path]:
    """`{N: path}` for every `<stem>-v<N>.json` directly under `out_dir`."""
    pattern = re.compile(re.escape(stem) + r"-v(\d+)\.json")
    found: dict[int, Path] = {}
    out = Path(out_dir)
    if not out.is_dir():
        return found
    for path in out.iterdir():
        match = pattern.fullmatch(path.name)
        if match and path.is_file():
            found[int(match.group(1))] = path
    return dict(sorted(found.items()))


def finding_files(out_dir: Path) -> list[tuple[str, int, Path]]:
    """`(assignment id, N, path)` for every `<id>-finding-v<N>.json`, sorted by (N, id)."""
    pattern = re.compile(r"(.+)-finding-v(\d+)\.json")
    found: list[tuple[str, int, Path]] = []
    out = Path(out_dir)
    if not out.is_dir():
        return found
    for path in out.iterdir():
        match = pattern.fullmatch(path.name)
        if match and path.is_file():
            found.append((match.group(1), int(match.group(2)), path))
    found.sort(key=lambda item: (item[1], item[0]))
    return found


# ── output ───────────────────────────────────────────────────────────────────


def cap_text(lines: list[str], limit: int = 3500) -> str:
    """Join `lines`; past `limit` characters keep whole lines and say how many were cut."""
    text = "\n".join(lines)
    if len(text) <= limit:
        return text
    budget = limit - 40
    kept: list[str] = []
    size = 0
    for line in lines:
        if len(line) > budget:
            line = line[: budget - 1] + "…"
        extra = len(line) + (1 if kept else 0)
        if size + extra > budget:
            break
        kept.append(line)
        size += extra
    more = len(lines) - len(kept)
    return "\n".join(kept + [f"… (+{more} more lines)"])


# ── the ledger ───────────────────────────────────────────────────────────────


def _new_claim(aid: str, index: int, pass_n: int) -> dict:
    return {
        "finding": aid,
        "index": index,
        "question_id": question_id_of(aid),
        "pass": pass_n,
        "text": "",
        "claimed_work_id": "",
        "claimed_title": "",
        "claimed_year": 0,
        "verdict": None,
        "passage": "",
        "retracted": False,
        "resolved": True,
        "work_key": None,
        "title_ratio": None,
        "flags": [],
        "citable": "unverified",
    }


def _authors(value: Any) -> list[str]:
    names: list[str] = []
    for item in as_list(value):
        name = as_str(item.get("name")) if isinstance(item, dict) else as_str(item)
        name = name.strip()
        if name:
            names.append(name)
    return names


def _merge_work(works: dict, key: str, work: dict, claim: str) -> None:
    entry = works.setdefault(
        key,
        {"openalex": "", "doi": "", "arxiv": "", "title": "", "year": 0, "venue": "", "authors": [], "claims": []},
    )
    cleaned = {
        "openalex": clean_openalex(work.get("openalex")),
        "doi": clean_doi(work.get("doi")),
        "arxiv": as_str(work.get("arxiv")).strip(),
        "title": as_str(work.get("title")).strip(),
        "venue": as_str(work.get("venue")).strip(),
    }
    for field in WORK_TEXT_FIELDS:
        if not entry[field] and cleaned[field]:
            entry[field] = cleaned[field]
    if not entry["year"]:
        entry["year"] = as_int(work.get("year"))
    if not entry["authors"]:
        entry["authors"] = _authors(work.get("authors"))
    if claim not in entry["claims"]:
        entry["claims"].append(claim)


def _claim_index(value: Any) -> Optional[int]:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.strip().isdigit():
        return int(value.strip())
    return None


def build_ledger(out_dir: Path) -> dict:
    """Every claim of the run with its audit verdict, resolved work and citable class."""
    out = Path(out_dir)
    claims: dict[str, dict] = {}
    works: dict[str, dict] = {}
    sources: list[str] = []

    for aid, pass_n, path in finding_files(out):
        data = as_dict(load_json(path))
        sources.append(path.name)
        for index, claim in enumerate(as_list(data.get("claims"))):
            claim = as_dict(claim)
            entry = _new_claim(aid, index, pass_n)
            entry["text"] = as_str(claim.get("text"))
            entry["claimed_work_id"] = as_str(claim.get("work_id"))
            entry["claimed_title"] = as_str(claim.get("title"))
            entry["claimed_year"] = as_int(claim.get("year"))
            claims[claim_key(aid, index)] = entry

    for pass_n, path in versioned_files(out, "audit").items():
        data = as_dict(load_json(path))
        sources.append(path.name)
        for audited in as_list(data.get("claims")):
            if not isinstance(audited, dict):
                continue
            aid = as_str(audited.get("finding")).strip()
            index = _claim_index(audited.get("index"))
            if not aid or index is None:
                continue
            key = claim_key(aid, index)
            entry = claims.get(key)
            if entry is None:
                entry = _new_claim(aid, index, pass_n)
                entry["flags"].append("no_finding")
                claims[key] = entry
            verdict = as_str(audited.get("verdict")).strip().lower()
            entry["verdict"] = verdict or None
            entry["passage"] = as_str(audited.get("passage"))
            entry["retracted"] = audited.get("retracted") is True
            entry["resolved"] = audited.get("resolved") is not False
            work = as_dict(audited.get("work"))
            key_of_work = work_key(work)
            if key_of_work is not None:
                entry["work_key"] = key_of_work
                _merge_work(works, key_of_work, work, key)

    for entry in claims.values():
        flags = [flag for flag in entry["flags"] if flag == "no_finding"]
        verdict = entry["verdict"]
        if verdict is None:
            flags.append("no_audit")
        if entry["retracted"]:
            flags.append("retracted")
        if entry["resolved"] is False:
            flags.append("unresolvable")
        if entry["work_key"] is None and verdict in ("supported", "partial"):
            flags.append("no_work")
        if entry["work_key"] is not None:
            ratio = title_ratio(entry["claimed_title"], works[entry["work_key"]]["title"])
            if ratio is not None:
                entry["title_ratio"] = round(ratio, 3)
                if ratio < TITLE_MISMATCH_THRESHOLD:
                    flags.append("title_mismatch")
        entry["flags"] = flags

        if verdict == "not_supported" or entry["retracted"] or entry["resolved"] is False:
            entry["citable"] = "forbidden"
        elif verdict in ("supported", "partial") and not {"title_mismatch", "no_work"} & set(flags):
            entry["citable"] = verdict
        else:
            entry["citable"] = "unverified"

    return {
        "sources": sorted(sources),
        "claims": dict(sorted(claims.items())),
        "works": dict(sorted(works.items())),
    }
