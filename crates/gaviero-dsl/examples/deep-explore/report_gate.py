"""Report gate of deep-explore.gaviero: `report_gate.py OUT_DIR`.

Runs as the `deep-explore-report` loop's `until command` after every
`synthesize` pass. Implements contract C4 of
plans/deep-explore-gates/deep-explore-gates-v1.md:

  1. rebuild OUT_DIR/ledger.json from every finding and audit;
  2. check report-draft.md (G1 sections, G2 unknown keys, G3 where each
     citable class may be cited, G4 uncited text, G5 closed questions
     left out);
  3. render report.md: claim keys [@q7-p2#0] become numbered works [n]
     and the references are printed from the ledger's resolved works;
  4. write report-audit.md and print the violations (they reach the next
     synthesize pass as GATE_FEEDBACK).

Exit codes: 0 = no violations, 1 = violations, 2 = input error.
Standard library only; Python 3.10 or newer.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gate_common  # noqa: E402
from gate_common import GateInputError, as_dict, as_int, as_list, as_str  # noqa: E402

REQUIRED = ["Answer", "Findings", "New avenues", "Contested and open", "How the field was explored", "Audit"]
MARKER_RE = re.compile(r"\[(@[^\[\]]+)\]")
KEY_RE = re.compile(r"@([A-Za-z0-9_.\-]+#\d+)")
HEADING_RE = re.compile(r"^(#{2,6})\s+(.*)$")
LIST_ITEM_RE = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s+")
FEEDBACK_TAIL = "Fix report-draft.md and keep every other passage unchanged."
OUTPUT_LIMIT = 3500


# ── parsing ──────────────────────────────────────────────────────────────────


def _section_name(text: str) -> Optional[str]:
    for name in REQUIRED:
        if re.match(rf"{re.escape(name)}\b", text.strip(), re.I):
            return name
    return None


def _is_references(text: str) -> bool:
    return bool(re.match(r"References\b", text.strip(), re.I))


def _parse_marker(content: str) -> list[tuple[Optional[str], str]]:
    """`(key, raw)` per `;`-separated part; key is None when malformed."""
    parts = []
    for part in content.split(";"):
        raw = part.strip()
        match = KEY_RE.fullmatch(raw)
        parts.append((match.group(1) if match else None, raw))
    return parts


def _scan(lines: list[str]) -> tuple[list[tuple[int, str]], list[dict]]:
    """The `##` headings and every citation with its line and context."""
    h2: list[tuple[int, str]] = []
    citations: list[dict] = []
    section: Optional[str] = None
    in_unverified = False
    for number, line in enumerate(lines, 1):
        heading = HEADING_RE.match(line)
        if heading:
            text = heading.group(2).strip()
            if len(heading.group(1)) == 2:
                h2.append((number, text))
                section = _section_name(text)
                in_unverified = False
            else:
                in_unverified = "unverified leads" in text.lower()
        for marker in MARKER_RE.finditer(line):
            for key, raw in _parse_marker(marker.group(1)):
                citations.append(
                    {"line": number, "key": key, "raw": raw, "section": section, "unverified_leads": in_unverified}
                )
    return h2, citations


def _section_body(lines: list[str], h2: list[tuple[int, str]], name: str) -> list[tuple[int, str]]:
    """`(line number, line)` of the first `##` section called `name`, heading excluded."""
    for i, (start, text) in enumerate(h2):
        if _section_name(text) == name and not _is_references(text):
            end = h2[i + 1][0] - 1 if i + 1 < len(h2) else len(lines)
            return [(k, lines[k - 1]) for k in range(start + 1, end + 1)]
    return []


def _snippet(text: str, width: int = 60) -> str:
    flat = " ".join(text.split())
    return flat if len(flat) <= width else flat[: width - 1] + "…"


def _violation(check: str, line: int, detail: str) -> dict:
    return {"check": check, "line": line, "detail": detail}


# ── checks ───────────────────────────────────────────────────────────────────


def _forbidden_reason(entry: dict) -> str:
    flags = as_list(entry.get("flags"))
    if "retracted" in flags:
        return "the source is retracted"
    if "unresolvable" in flags:
        return "the source does not resolve"
    return "the audit verdict is not_supported"


def _unverified_reason(entry: dict) -> str:
    flags = as_list(entry.get("flags"))
    if "title_mismatch" in flags:
        return "the claimed title does not match the resolved work"
    if "no_work" in flags:
        return "the verifier recorded no resolved work"
    if "no_audit" in flags:
        return "no audit verdict"
    return f"audit verdict {as_str(entry.get('verdict')) or 'none'}"


def _g4(lines: list[str], h2: list[tuple[int, str]]) -> list[dict]:
    violations: list[dict] = []

    blocks: list[list] = []
    current: Optional[list] = None
    for number, line in _section_body(lines, h2, "Findings"):
        if not line.strip() or HEADING_RE.match(line):
            current = None
            continue
        if current is None or LIST_ITEM_RE.match(line):
            current = [number, line]
            blocks.append(current)
        else:
            current[1] += "\n" + line
    for number, text in blocks:
        if not MARKER_RE.search(text):
            violations.append(
                _violation("G4", number, f"uncited text under ## Findings: '{_snippet(text)}' — cite at least one claim key")
            )

    subsections: list[list] = []
    current = None
    for number, line in _section_body(lines, h2, "New avenues"):
        heading = HEADING_RE.match(line)
        if heading and len(heading.group(1)) <= 3:
            current = [number, heading.group(2).strip(), ""] if len(heading.group(1)) == 3 else None
            if current is not None:
                subsections.append(current)
            continue
        if current is not None:
            current[2] += "\n" + line
    for number, title, text in subsections:
        if not MARKER_RE.search(text):
            violations.append(
                _violation("G4", number, f"### {_snippet(title)} under ## New avenues cites no claim key")
            )
    return violations


def check_draft(text: str, ledger: dict, frontier: dict) -> list[dict]:
    """Every G1-G5 violation of `text`, sorted by line."""
    lines = text.split("\n")
    claims = as_dict(ledger.get("claims"))
    violations: list[dict] = []
    h2, citations = _scan(lines)

    # G1. Sections.
    seen: set[str] = set()
    last = -1
    for number, heading in h2:
        if _is_references(heading):
            violations.append(_violation("G1", number, "remove ## References: the report gate writes the references"))
            continue
        name = _section_name(heading)
        if name is None:
            violations.append(
                _violation("G1", number, f"unexpected ## {_snippet(heading)}; the only ## sections are: {', '.join(REQUIRED)}")
            )
            continue
        if name in seen:
            violations.append(_violation("G1", number, f"duplicate ## {name}"))
            continue
        seen.add(name)
        index = REQUIRED.index(name)
        if index < last:
            violations.append(_violation("G1", number, f"## {name} is out of order; the order is: {', '.join(REQUIRED)}"))
        last = max(last, index)
    for name in REQUIRED:
        if name not in seen:
            violations.append(_violation("G1", 1, f"missing ## {name}"))

    # G2 and G3. Keys and where they may be cited.
    for cite in citations:
        key = cite["key"]
        if key is None:
            violations.append(
                _violation("G2", cite["line"], f"malformed citation '{cite['raw']}': write [@<finding id>#<claim index>]")
            )
            continue
        entry = claims.get(key)
        if entry is None:
            violations.append(_violation("G2", cite["line"], f"[@{key}] is not a claim key of ledger.json"))
            continue
        citable = entry.get("citable")
        if citable == "forbidden":
            violations.append(_violation("G3", cite["line"], f"[@{key}] is forbidden ({_forbidden_reason(entry)}): remove it"))
        elif citable == "unverified" and not cite["unverified_leads"]:
            violations.append(
                _violation(
                    "G3",
                    cite["line"],
                    f"[@{key}] is unverified ({_unverified_reason(entry)}): cite it only under ### Unverified leads",
                )
            )

    # G4. Uncited text.
    violations.extend(_g4(lines, h2))

    # G5. Closed questions with supported claims must be cited.
    cited_questions = {
        as_str(claims[c["key"]].get("question_id"))
        for c in citations
        if c["key"] in claims and claims[c["key"]].get("citable") in ("supported", "partial")
    }
    supported_by_question: dict[str, list[str]] = {}
    for key, entry in claims.items():
        if entry.get("citable") == "supported":
            supported_by_question.setdefault(as_str(entry.get("question_id")), []).append(key)
    for q in as_list(frontier.get("questions")):
        q = as_dict(q)
        qid = as_str(q.get("id"))
        if as_str(q.get("status")).strip().lower() != "closed":
            continue
        keys = supported_by_question.get(qid)
        if keys and qid not in cited_questions:
            violations.append(
                _violation(
                    "G5",
                    1,
                    f"closed question {qid} has supported claims ({', '.join(keys[:3])}) but none is cited",
                )
            )

    violations.sort(key=lambda v: (v["line"], v["check"]))
    return violations


# ── rendering ────────────────────────────────────────────────────────────────


def _reference(n: int, work: dict) -> str:
    names = [as_str(a) for a in as_list(work.get("authors")) if as_str(a)]
    authors = ", ".join(names[:3]) + (" et al." if len(names) > 3 else "")
    year = as_int(work.get("year"))
    venue_year = ", ".join(x for x in [as_str(work.get("venue")), str(year) if year else ""] if x)
    if as_str(work.get("doi")):
        ident = "doi:" + as_str(work.get("doi"))
    elif as_str(work.get("openalex")):
        ident = "OpenAlex " + as_str(work.get("openalex"))
    elif as_str(work.get("arxiv")):
        ident = "arXiv:" + as_str(work.get("arxiv"))
    else:
        ident = ""
    parts = [p.rstrip(".") for p in [authors, as_str(work.get("title")), venue_year, ident] if p and p.rstrip(".")]
    return f"{n}. " + ". ".join(parts) + "."


def render_report(text: str, ledger: dict) -> tuple[str, dict, list[str]]:
    """(report markdown with references, claim key -> work number, reference lines)."""
    claims = as_dict(ledger.get("claims"))
    works = as_dict(ledger.get("works"))

    kept: list[str] = []
    skipping = False
    for line in text.split("\n"):
        heading = HEADING_RE.match(line)
        if heading and len(heading.group(1)) == 2:
            skipping = _is_references(heading.group(2))
            if skipping:
                continue
        if not skipping:
            kept.append(line)
    body = "\n".join(kept)

    work_numbers: dict[str, int] = {}
    key_numbers: dict[str, int] = {}

    def replace(match: "re.Match[str]") -> str:
        numbers: list[int] = []
        extras: list[str] = []
        for key, _raw in _parse_marker(match.group(1)):
            entry = claims.get(key) if key else None
            if entry is None:
                extras.append("[?]")
                continue
            wk = entry.get("work_key")
            if wk and wk in works:
                if wk not in work_numbers:
                    work_numbers[wk] = len(work_numbers) + 1
                numbers.append(work_numbers[wk])
                key_numbers[key] = work_numbers[wk]
            else:
                extras.append(f"[unverified: {as_str(entry.get('claimed_work_id')) or '?'}]")
        pieces: list[str] = []
        if numbers:
            pieces.append("[" + ", ".join(str(x) for x in sorted(set(numbers))) + "]")
        for extra in extras:
            if extra not in pieces:
                pieces.append(extra)
        return " ".join(pieces)

    rendered = MARKER_RE.sub(replace, body)
    refs = [_reference(n, as_dict(works[wk])) for wk, n in sorted(work_numbers.items(), key=lambda item: item[1])]
    report = rendered.rstrip("\n") + "\n\n## References\n\n" + "\n".join(refs) + ("\n" if refs else "")
    return report, key_numbers, refs


# ── audit file ───────────────────────────────────────────────────────────────


def _cell(text: Any) -> str:
    return as_str(text).replace("|", "\\|")


def _audit_md(violations: list[dict], text: str, ledger: dict, numbers: dict) -> str:
    claims = as_dict(ledger.get("claims"))
    works = as_dict(ledger.get("works"))
    result = "Result: PASS" if not violations else f"Result: FAIL ({len(violations)} violations)"
    out = ["# Report gate audit", "", result, "", "## Violations", ""]
    out += [f"- {v['check']} line {v['line']}: {v['detail']}" for v in violations] or ["- none"]

    out += ["", "## Citations", "", "| key | citable | work | n |", "|---|---|---|---|"]
    _, citations = _scan(text.split("\n"))
    listed: set[str] = set()
    for cite in citations:
        label = cite["key"] or cite["raw"]
        if label in listed:
            continue
        listed.add(label)
        if cite["key"] is None:
            citable, work = "malformed", ""
        elif cite["key"] not in claims:
            citable, work = "orphan", ""
        else:
            entry = claims[cite["key"]]
            citable, work = as_str(entry.get("citable")), as_str(entry.get("work_key"))
        out.append(f"| {_cell(label)} | {citable} | {_cell(work)} | {numbers.get(cite['key'], '')} |")

    out += ["", "## Title mismatches", ""]
    mismatches = [
        f"- {key}: claimed \"{as_str(e.get('claimed_title'))}\" vs resolved "
        f"\"{as_str(as_dict(works.get(e.get('work_key'))).get('title'))}\" (ratio {e.get('title_ratio')})"
        for key, e in claims.items()
        if "title_mismatch" in as_list(e.get("flags"))
    ]
    out += mismatches or ["- none"]

    by_class = {name: 0 for name in ("supported", "partial", "unverified", "forbidden")}
    for entry in claims.values():
        name = as_str(entry.get("citable"))
        if name in by_class:
            by_class[name] += 1
    cited_keys = {c["key"] for c in citations if c["key"] in claims}
    out += [
        "",
        "## Counts",
        "",
        f"- claims in the ledger: {len(claims)} (supported {by_class['supported']}, partial {by_class['partial']}, "
        f"unverified {by_class['unverified']}, forbidden {by_class['forbidden']})",
        f"- cited keys: {len(cited_keys)}",
        f"- works cited: {len(set(numbers.values()))}",
        "",
    ]
    return "\n".join(out)


# ── main ─────────────────────────────────────────────────────────────────────


def _utf8_stdout() -> None:
    # gaviero reads the probe's output as UTF-8; a Windows pipe defaults to
    # the ANSI code page, which mangles "—" and cannot encode most scripts.
    reconfigure = getattr(sys.stdout, "reconfigure", None)
    if reconfigure is not None:
        reconfigure(encoding="utf-8", errors="replace")


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="report_gate.py",
        description="Report gate of deep-explore.gaviero: checks report-draft.md, writes report.md and "
        "report-audit.md. Exit 0 = no violations, 1 = violations, 2 = input error.",
    )
    parser.add_argument("out_dir", help="the run's OUT_DIR")
    return parser


def main(argv: Optional[list] = None) -> int:
    _utf8_stdout()
    args = _parser().parse_args(argv)
    try:
        return _run(Path(args.out_dir))
    except GateInputError as exc:
        print(f"REPORT GATE: INPUT ERROR: {exc}")
        return 2


def _run(out: Path) -> int:
    if not out.is_dir():
        raise GateInputError(f"{out}: not a directory")
    draft = out / "report-draft.md"
    if not draft.is_file():
        raise GateInputError("report-draft.md: file not found")
    if not gate_common.versioned_files(out, "audit"):
        raise GateInputError("no audit-v*.json in OUT_DIR")
    frontiers = gate_common.versioned_files(out, "frontier")
    if not frontiers:
        raise GateInputError("no frontier-v*.json in OUT_DIR")
    frontier = as_dict(gate_common.load_json(frontiers[max(frontiers)]))
    try:
        text = draft.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise GateInputError(f"report-draft.md: cannot read ({exc})") from None
    text = text.lstrip("﻿").replace("\r\n", "\n")

    ledger = gate_common.build_ledger(out)
    gate_common.write_json(out / "ledger.json", ledger)

    violations = check_draft(text, ledger, frontier)
    rendered, numbers, _refs = render_report(text, ledger)
    if violations:
        rendered = f"> REPORT GATE FAILED: {len(violations)} violations — see report-audit.md\n\n" + rendered
    gate_common.write_text(out / "report.md", rendered)
    gate_common.write_text(out / "report-audit.md", _audit_md(violations, text, ledger, numbers))

    if not violations:
        print("REPORT GATE: PASS\nwrote: report.md, report-audit.md, ledger.json")
        return 0
    lines = [f"REPORT GATE: FAIL ({len(violations)} violations)"]
    lines += [f"{v['check']} line {v['line']}: {v['detail']}" for v in violations]
    print(gate_common.cap_text(lines, limit=OUTPUT_LIMIT - len(FEEDBACK_TAIL) - 1) + "\n" + FEEDBACK_TAIL)
    return 1


if __name__ == "__main__":
    sys.exit(main())
