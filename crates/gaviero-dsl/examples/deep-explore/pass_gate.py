"""Pass gate of deep-explore.gaviero: `pass_gate.py OUT_DIR ITER`.

Runs as the exploration loop's `until command` after every pass N.
Implements contract C3 of plans/deep-explore-gates/deep-explore-gates-v1.md:

  1. rebuild OUT_DIR/ledger.json from every finding and audit;
  2. apply the mechanical merge of pass N to frontier-vN.json and write
     frontier-v(N+1).json (always, also when the loop stops);
  3. recount the audit, apply the stop rules, write metrics-vN.json and
     append dead ends / dropped claims to archive.md;
  4. print a short report (it reaches the next pass as GATE_FEEDBACK).

Exit codes: 0 = stop the loop, 1 = run another pass, 2 = input error.
Standard library only; Python 3.10 or newer.
"""

from __future__ import annotations

import argparse
import copy
import re
import sys
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gate_common  # noqa: E402
from gate_common import GateInputError, as_dict, as_int, as_list, as_str  # noqa: E402

KINDS = ("core", "mechanism", "method", "contrarian", "adjacent", "gap", "bridge", "blind_spot")
STATUSES = ("open", "active", "closed", "dead", "parked")
SCORE_KEYS = ("relevance", "gain", "novelty", "tractability")
QUESTION_FIELDS = (
    "id", "question", "kind", "operator", "parent", "depth", "status",
    "scores", "priority", "visits", "supported_claims", "why",
)
# Closed and dead questions keep only these. `kind` stays so that rule S3
# (every core question closed) can still see which closed questions are core.
CLOSED_FIELDS = ("id", "question", "kind", "status", "operator", "parent", "depth", "supported_claims")
FRONTIER_KEYS = ("pass", "collection", "questions", "leading_claims", "metrics")
MERGE_LOG_KEYS = (
    "closed", "reopened", "narrow", "dead", "children_added", "children_skipped",
    "restored", "dropped_claims", "unknown_assignments", "missing_findings", "truncated",
)
COUNT_KEYS = ("claims", "supported", "partial", "not_supported", "cannot_verify", "retracted", "unresolvable")

MAX_DEPTH = 3
MAX_CREATED = 40
QUESTION_CHARS = 250
WHY_CHARS = 300
LEADING_CLAIMS = 5
LEADING_CHARS = 200
SAFE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*")


# ── helpers ──────────────────────────────────────────────────────────────────


def _truncate(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[: limit - 1] + "…"


def _status(q: dict) -> str:
    status = as_str(q.get("status")).strip().lower()
    return status if status in STATUSES else "open"


def _claims_of(ledger: dict, aid: str, citable: str) -> list[str]:
    found = [
        (entry.get("index", 0), key)
        for key, entry in as_dict(ledger.get("claims")).items()
        if entry.get("finding") == aid and entry.get("citable") == citable
    ]
    return [key for _, key in sorted(found)]


def _bracket(ids: list) -> str:
    return "[" + ",".join(as_str(i) for i in ids) + "]"


def _yes(flag: bool) -> str:
    return "yes" if flag else "no"


def _novel(audit: Optional[dict]) -> Optional[int]:
    value = as_dict(as_dict(audit).get("metrics")).get("novel_children")
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    return None


# ── recount ──────────────────────────────────────────────────────────────────


def recount(audit: dict) -> dict:
    """Claim counts recomputed from the audit's own claim list."""
    counts = {key: 0 for key in COUNT_KEYS}
    for c in as_list(as_dict(audit).get("claims")):
        if not isinstance(c, dict):
            continue
        counts["claims"] += 1
        verdict = as_str(c.get("verdict")).strip().lower()
        if verdict in gate_common.VERDICTS:
            counts[verdict] += 1
        if c.get("retracted") is True:
            counts["retracted"] += 1
        if c.get("resolved") is False:
            counts["unresolvable"] += 1
    return counts


# ── merge ────────────────────────────────────────────────────────────────────


def merge_frontier(
    n: int,
    frontier: dict,
    prev_frontier: Optional[dict],
    assignments: list,
    findings: dict,
    audit: dict,
    ledger: dict,
) -> tuple[dict, dict, list[str]]:
    """Apply rules M1-M11; return (frontier-v(N+1), merge log, archive lines)."""
    fr = copy.deepcopy(as_dict(frontier))
    log: dict[str, list] = {key: [] for key in MERGE_LOG_KEYS}
    archive: list[str] = []

    questions: list[dict] = []
    by_id: dict[str, dict] = {}
    for q in as_list(fr.get("questions")):
        qid = as_str(as_dict(q).get("id")).strip()
        if qid and qid not in by_id:
            questions.append(q)
            by_id[qid] = q

    # M1. Restore question ids the curator lost.
    if n >= 2 and isinstance(prev_frontier, dict):
        for q in as_list(prev_frontier.get("questions")):
            qid = as_str(as_dict(q).get("id")).strip()
            if qid and qid not in by_id:
                restored = copy.deepcopy(q)
                questions.append(restored)
                by_id[qid] = restored
                log["restored"].append(qid)

    # M2. Start state.
    created_total = max(as_int(as_dict(fr.get("metrics")).get("created_total")), len(questions))
    status0 = {as_str(q.get("id")): _status(q) for q in questions}

    fidelity: dict[str, str] = {}
    for entry in as_list(as_dict(audit).get("fidelity")):
        entry = as_dict(entry)
        fidelity[as_str(entry.get("finding"))] = as_str(entry.get("verdict")).strip().lower()

    lanes: list[tuple[str, str]] = []
    for a in assignments:
        aid = as_str(as_dict(a).get("id")).strip()
        lanes.append((aid, gate_common.question_id_of(aid)))

    # M3. Lane outcomes.
    for aid, qid in lanes:
        if aid not in findings:
            log["missing_findings"].append(aid)
        q = by_id.get(qid)
        if q is None:
            log["unknown_assignments"].append(aid)
            continue
        finding = as_dict(findings.get(aid))
        fid = fidelity.get(aid) or as_str(finding.get("fidelity")).strip().lower() or "exact"
        sup = _claims_of(ledger, aid, "supported")
        par = _claims_of(ledger, aid, "partial")
        q["visits"] = as_int(q.get("visits")) + 1
        supported = [as_str(k) for k in as_list(q.get("supported_claims"))]

        if status0.get(qid) in ("closed", "dead", "parked"):
            supported.extend(k for k in sup if k not in supported)
            q["supported_claims"] = supported
            continue

        if fid == "substituted":
            q["status"] = "open"
            log["reopened"].append(qid)
        elif sup:
            supported.extend(k for k in sup if k not in supported)
            q["status"] = "closed"
            log["closed"].append(qid)
        elif par:
            q["status"] = "open"
            log["narrow"].append(qid)
        elif q["visits"] >= 2 and not supported:
            q["status"] = "dead"
            log["dead"].append(qid)
            archive.append(
                f"- {qid} (pass {n}, pass gate): {as_str(q.get('question'))} — "
                f"no usable evidence after {q['visits']} lanes"
            )
        else:
            q["status"] = "open"
        q["supported_claims"] = supported

    # M4. Drop retracted or unresolvable claims.
    bad: dict[str, str] = {}
    for key, entry in as_dict(ledger.get("claims")).items():
        flags = as_list(entry.get("flags"))
        if "retracted" in flags:
            bad[key] = "retracted"
        elif "unresolvable" in flags:
            bad[key] = "unresolvable"
    for q in questions:
        supported = [as_str(k) for k in as_list(q.get("supported_claims"))]
        kept = [k for k in supported if k not in bad]
        if len(kept) == len(supported):
            continue
        for key in supported:
            if key in bad and key not in log["dropped_claims"]:
                log["dropped_claims"].append(key)
                text = as_str(as_dict(ledger["claims"].get(key)).get("text"))[:120]
                archive.append(f"- claim {key} dropped (pass {n}, pass gate): {bad[key]}; {text}")
        q["supported_claims"] = kept
        if _status(q) == "closed" and not kept:
            q["status"] = "open"
            log["reopened"].append(as_str(q.get("id")))

    # M5. Nothing stays active between passes.
    for q in questions:
        if as_str(q.get("status")).strip().lower() == "active":
            q["status"] = "open"

    # M6. Children of this pass's findings.
    seen = {gate_common.normalize_title(q.get("question")) for q in questions}
    seen.discard("")
    numbers = [int(m.group(1)) for m in (re.fullmatch(r"q(\d+)", as_str(q.get("id"))) for q in questions) if m]
    next_number = (max(numbers) + 1) if numbers else 1
    for aid, qid in lanes:
        finding = findings.get(aid)
        if not isinstance(finding, dict):
            continue
        parent = by_id.get(qid)
        for c in as_list(finding.get("children")):
            c = as_dict(c)
            text = as_str(c.get("question")).strip()
            if not text:
                continue
            if parent is None:
                log["children_skipped"].append({"question": text, "reason": "unknown_parent"})
                continue
            depth = as_int(parent.get("depth")) + 1
            if depth > MAX_DEPTH:
                log["children_skipped"].append({"question": text, "reason": "depth"})
                continue
            if created_total >= MAX_CREATED:
                log["children_skipped"].append({"question": text, "reason": "cap"})
                continue
            norm = gate_common.normalize_title(text)
            if norm in seen:
                log["children_skipped"].append({"question": text, "reason": "duplicate"})
                continue
            kind = as_str(c.get("kind")).strip()
            new_id = f"q{next_number}"
            next_number += 1
            new = {
                "id": new_id,
                "question": text,
                "kind": kind if kind in KINDS else "gap",
                "operator": "child",
                "parent": qid,
                "depth": depth,
                "status": "open",
                "scores": {key: 0 for key in SCORE_KEYS},
                "priority": 0,
                "visits": 0,
                "supported_claims": [],
                "why": as_str(c.get("why")),
            }
            questions.append(new)
            by_id[new_id] = new
            seen.add(norm)
            created_total += 1
            log["children_added"].append(new_id)

    # M8 (then M7 on the normalised entries) and M9.
    out_questions: list[dict] = []
    for q in questions:
        status = _status(q)
        qid = as_str(q.get("id"))
        if status in ("closed", "dead", "parked"):
            fields = CLOSED_FIELDS + (("why",) if status == "parked" else ())
        else:
            fields = QUESTION_FIELDS
        nq: dict[str, Any] = {}
        for field in fields:
            value = q.get(field)
            if field in ("depth", "visits", "priority"):
                nq[field] = as_int(value)
            elif field == "supported_claims":
                nq[field] = [as_str(k) for k in as_list(value)]
            elif field == "scores":
                scores = as_dict(value)
                nq[field] = {key: as_int(scores.get(key)) for key in SCORE_KEYS}
            elif field == "status":
                nq[field] = status
            else:
                nq[field] = as_str(value)
        if "scores" in nq:
            nq["priority"] = sum(nq["scores"].values()) - nq["depth"]
        truncated = False
        if len(nq["question"]) > QUESTION_CHARS:
            nq["question"] = _truncate(nq["question"], QUESTION_CHARS)
            truncated = True
        if "why" in nq and len(nq["why"]) > WHY_CHARS:
            nq["why"] = _truncate(nq["why"], WHY_CHARS)
            truncated = True
        if truncated:
            log["truncated"].append(qid)
        out_questions.append(nq)

    leading_in = [as_str(c) for c in as_list(fr.get("leading_claims"))]
    leading = [_truncate(c, LEADING_CHARS) for c in leading_in[:LEADING_CLAIMS]]
    if leading != leading_in:
        log["truncated"].append("leading_claims")

    # M10. Metrics and shape.
    def count(status: str) -> int:
        return sum(1 for q in out_questions if q["status"] == status)

    merged = {
        "pass": n + 1,
        "collection": as_str(fr.get("collection")),
        "questions": out_questions,
        "leading_claims": leading,
        "metrics": {
            "open": count("open"),
            "closed": count("closed"),
            "dead": count("dead"),
            "parked": count("parked"),
            "created_total": created_total,
        },
    }
    return merged, log, archive


# ── stop rules ───────────────────────────────────────────────────────────────


def evaluate_stop(
    n: int,
    assignments: list,
    merged: dict,
    counts: dict,
    novel: Optional[int],
    prev_counts: Optional[dict],
    prev_novel: Optional[int],
) -> dict:
    """Rules S0-S3 and the cannot_verify blocker, on the merged frontier."""
    questions = [as_dict(q) for q in as_list(merged.get("questions"))]
    open_qs = [q for q in questions if q.get("status") == "open"]
    high_open = [
        q for q in open_qs
        if as_int(q.get("priority")) >= 6 and as_int(as_dict(q.get("scores")).get("tractability")) >= 1
    ]
    core = [q for q in questions if q.get("kind") == "core"]

    s0 = len(assignments) == 0
    s1 = novel == 0 and not high_open
    s2 = (
        n >= 2
        and prev_counts is not None
        and novel is not None
        and prev_novel is not None
        and counts["supported"] < 2
        and novel < 2
        and prev_counts["supported"] < 2
        and prev_novel < 2
    )
    s3 = (
        len(core) >= 1
        and all(q.get("status") == "closed" for q in core)
        and all(as_int(as_dict(q.get("scores")).get("relevance")) <= 1 for q in open_qs)
    )
    blocked = counts["cannot_verify"] >= 2
    stop = s0 or s3 or ((s1 or s2) and not blocked)

    fired: list[str] = []
    if s0:
        fired.append("S0")
    if (s1 or s2) and not blocked:
        fired.extend(name for name, flag in (("S1", s1), ("S2", s2)) if flag)
    if s3:
        fired.append("S3")

    reasons: list[str] = []
    if s0:
        reasons.append("S0: the curator assigned no lane this pass")
    if s3:
        reasons.append(f"S3: all {len(core)} core questions are closed and every open question has relevance <= 1")
    if s1 and not blocked:
        reasons.append("S1: no open question has priority >= 6 and tractability >= 1, and novel_children = 0")
    if s2 and not blocked:
        reasons.append(
            f"S2: supported {counts['supported']}/{prev_counts['supported'] if prev_counts else '-'} "
            f"and novel_children {novel}/{prev_novel} in passes {n}/{n - 1}, all < 2"
        )
    if (s1 or s2) and blocked and not stop:
        reasons.append(f"S1/S2 held but cannot_verify = {counts['cannot_verify']} >= 2 blocks them")
    if not reasons:
        reasons.append(
            f"no stop rule held: {len(high_open)} open questions with priority >= 6, "
            f"supported = {counts['supported']}, novel_children = {novel if novel is not None else 'missing'}"
        )
    return {
        "S0": s0,
        "S1": s1,
        "S2": s2,
        "S3": s3,
        "blocked": blocked,
        "decision": "stop" if stop else "continue",
        "reason": "; ".join(reasons),
        "fired": fired,
    }


# ── main ─────────────────────────────────────────────────────────────────────


def _optional_json(path: Path) -> Any:
    if not path.is_file():
        return None
    try:
        return gate_common.load_json(path)
    except GateInputError:
        return None


def _positive_int(value: str) -> int:
    try:
        number = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not an integer: {value!r}") from None
    if number < 1:
        raise argparse.ArgumentTypeError(f"must be >= 1: {value!r}")
    return number


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="pass_gate.py",
        description="Pass gate of deep-explore.gaviero: ledger, frontier merge and stop rule. "
        "Exit 0 = stop the loop, 1 = run another pass, 2 = input error.",
    )
    parser.add_argument("out_dir", help="the run's OUT_DIR")
    parser.add_argument("iter", type=_positive_int, help="the pass that just ended (ITER)")
    return parser


def _utf8_stdout() -> None:
    # gaviero reads the probe's output as UTF-8; a Windows pipe defaults to
    # the ANSI code page, which mangles "—" and cannot encode most scripts.
    reconfigure = getattr(sys.stdout, "reconfigure", None)
    if reconfigure is not None:
        reconfigure(encoding="utf-8", errors="replace")


def main(argv: Optional[list] = None) -> int:
    _utf8_stdout()
    args = _parser().parse_args(argv)
    out = Path(args.out_dir)
    n = args.iter
    try:
        return _run(out, n)
    except GateInputError as exc:
        msg = str(exc)
        try:
            gate_common.write_json(out / f"metrics-v{n}.json", {"pass": n, "error": msg})
        except OSError:
            pass
        print(f"PASS GATE v{n}: INPUT ERROR: {msg}")
        return 2


def _run(out: Path, n: int) -> int:
    if not out.is_dir():
        raise GateInputError(f"{out}: not a directory")
    assignments = gate_common.load_json(out / f"assignments-v{n}.json")
    if not isinstance(assignments, list):
        raise GateInputError(f"assignments-v{n}.json: expected a JSON array")
    frontier = gate_common.load_json(out / f"frontier-v{n}.json")
    if not isinstance(frontier, dict):
        raise GateInputError(f"frontier-v{n}.json: expected a JSON object")
    audit = gate_common.load_json(out / f"audit-v{n}.json")
    if not isinstance(audit, dict):
        raise GateInputError(f"audit-v{n}.json: expected a JSON object")
    prev_frontier = _optional_json(out / f"frontier-v{n - 1}.json") if n >= 2 else None
    prev_audit = _optional_json(out / f"audit-v{n - 1}.json") if n >= 2 else None
    if not isinstance(prev_frontier, dict):
        prev_frontier = None
    if not isinstance(prev_audit, dict):
        prev_audit = None

    findings: dict[str, dict] = {}
    for a in assignments:
        aid = as_str(as_dict(a).get("id")).strip()
        if not SAFE_ID.fullmatch(aid):
            continue
        data = _optional_json(out / f"{aid}-finding-v{n}.json")
        if isinstance(data, dict):
            findings[aid] = data

    ledger = gate_common.build_ledger(out)
    gate_common.write_json(out / "ledger.json", ledger)

    merged, merge_log, archive_lines = merge_frontier(
        n, frontier, prev_frontier, assignments, findings, audit, ledger
    )

    counts = recount(audit)
    prev_counts = recount(prev_audit) if prev_audit is not None else None
    novel = _novel(audit)
    prev_novel = _novel(prev_audit) if prev_audit is not None else None
    verifier_metrics = as_dict(audit.get("metrics"))
    mismatches = [
        f"verifier {key}={verifier_metrics[key]} vs recount {counts[key]}"
        for key in COUNT_KEYS
        if key in verifier_metrics and verifier_metrics[key] != counts[key]
    ]

    stop = evaluate_stop(n, assignments, merged, counts, novel, prev_counts, prev_novel)

    gate_common.write_json(out / f"frontier-v{n + 1}.json", merged)
    gate_common.write_json(
        out / f"metrics-v{n}.json",
        {
            "pass": n,
            "decision": stop["decision"],
            "reason": stop["reason"],
            "rules": {key: stop[key] for key in ("S0", "S1", "S2", "S3", "blocked")},
            "counts": counts,
            "novel_children": novel,
            "verifier_metrics": verifier_metrics,
            "mismatches": mismatches,
            "merge": merge_log,
        },
    )
    if archive_lines:
        with open(out / "archive.md", "a", encoding="utf-8", newline="\n") as handle:
            handle.write(f"\n## Pass {n} (pass gate)\n" + "\n".join(archive_lines) + "\n")

    if stop["decision"] == "stop":
        head = f"PASS GATE v{n}: STOP ({', '.join(stop['fired'])}) — {stop['reason']}"
    else:
        head = f"PASS GATE v{n}: CONTINUE — {stop['reason']}"
    lines = [
        head,
        f"rules: S0={_yes(stop['S0'])} S1={_yes(stop['S1'])} S2={_yes(stop['S2'])} "
        f"S3={_yes(stop['S3'])} blocked={_yes(stop['blocked'])} (cannot_verify={counts['cannot_verify']})",
        "audit: " + " ".join(f"{key}={counts[key]}" for key in COUNT_KEYS)
        + f" novel_children={novel if novel is not None else 'missing'}",
    ]
    lines.extend(f"mismatch: {m}" for m in mismatches)
    lines.append(
        f"merge: closed={_bracket(merge_log['closed'])} reopened={_bracket(merge_log['reopened'])} "
        f"narrow={_bracket(merge_log['narrow'])} dead={_bracket(merge_log['dead'])} "
        f"children=+{len(merge_log['children_added'])} {_bracket(merge_log['children_added'])} "
        f"skipped={len(merge_log['children_skipped'])} restored={_bracket(merge_log['restored'])} "
        f"dropped={len(merge_log['dropped_claims'])}"
    )
    if merge_log["unknown_assignments"] or merge_log["missing_findings"]:
        lines.append(
            f"notes: unknown assignments {_bracket(merge_log['unknown_assignments'])}, "
            f"missing findings {_bracket(merge_log['missing_findings'])}"
        )
    lines.append(f"wrote: frontier-v{n + 1}.json (curator: edit it in place), ledger.json, metrics-v{n}.json")
    lines.append("This is the pass gate's report for the next pass, not an error.")
    print(gate_common.cap_text(lines))
    return 0 if stop["decision"] == "stop" else 1


if __name__ == "__main__":
    sys.exit(main())
