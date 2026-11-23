"""Tests for report_gate.py (contract C4)."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import helpers  # noqa: E402
import report_gate  # noqa: E402

SCRIPT = helpers.TOOLS_DIR / "report_gate.py"

FOUR_AUTHORS = ("Ann Author", "Bo Builder", "Cy Coder", "Di Doer")

GOOD_DRAFT = """# Report

## Answer

The answer is X [@q1-p1#0].

## Findings

### q1 — Things

Claim A holds [@q1-p1#0].

- Evidence: a single study [@q1-p1#0; @q1-p1#0].

## New avenues

### Avenue one

Promising because of [@q1-p1#0].

## Contested and open

Disputes remain.

### Unverified leads

- Claim B, unverified [@q2-p1#0].

## How the field was explored

Two passes, three lanes.

## Audit

Three claims checked.
"""


def make_run(out: Path) -> None:
    """q1 closed with a supported claim on W1, q2-p1#0 unverified (W2), q3-p2#0 forbidden."""
    helpers.write_pass(
        out,
        1,
        findings=[
            helpers.finding("q1-p1", [helpers.claim("Claim A")]),
            helpers.finding("q2-p1", [helpers.claim("Claim B", work_id="W2", title="Another paper")]),
        ],
        audit=helpers.audit(
            1,
            [
                helpers.audit_claim("q1-p1", 0, "supported", work=helpers.work(doi="10.1/x", authors=FOUR_AUTHORS)),
                helpers.audit_claim(
                    "q2-p1", 0, "cannot_verify", work=helpers.work(openalex="W2", title="Another paper", authors=("Ed Else",))
                ),
            ],
        ),
    )
    helpers.write_pass(
        out,
        2,
        findings=[helpers.finding("q3-p2", [helpers.claim("Claim C", work_id="W3", title="Third paper")])],
        audit=helpers.audit(
            2, [helpers.audit_claim("q3-p2", 0, "not_supported", work=helpers.work(openalex="W3", title="Third paper"))]
        ),
    )
    helpers.write_json(
        out,
        "frontier-v3.json",
        helpers.frontier(
            3,
            [
                helpers.question("q1", status="closed", supported_claims=["q1-p1#0"]),
                helpers.question("q2"),
                helpers.question("q3"),
            ],
        ),
    )


class ReportGateTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.out = Path(self._tmp.name)
        make_run(self.out)

    def tearDown(self):
        self._tmp.cleanup()

    def gate(self, draft: str) -> tuple[int, str]:
        (self.out / "report-draft.md").write_text(draft, encoding="utf-8")
        return helpers.run_main(report_gate, [self.out])

    def violations(self, draft: str) -> list[dict]:
        ledger = report_gate.gate_common.build_ledger(self.out)
        frontier = helpers.read_json(self.out, "frontier-v3.json")
        return report_gate.check_draft(draft, ledger, frontier)

    def read(self, name: str) -> str:
        return (self.out / name).read_text(encoding="utf-8")


class PassingDraftTests(ReportGateTestCase):
    def test_good_draft_passes_and_renders_numbered_references(self):
        code, out = self.gate(GOOD_DRAFT)
        self.assertEqual(code, 0, out)
        self.assertTrue(out.startswith("REPORT GATE: PASS"))
        report = self.read("report.md")
        self.assertNotIn("[@", report)
        self.assertNotIn("REPORT GATE FAILED", report)
        self.assertIn("Claim A holds [1].", report)
        self.assertIn("a single study [1].", report)
        self.assertIn("Claim B, unverified [2].", report)
        last_h2 = [line for line in report.splitlines() if line.startswith("## ")][-1]
        self.assertEqual(last_h2, "## References")
        refs = report.split("## References", 1)[1].strip().splitlines()
        self.assertEqual(refs[0], "1. Ann Author, Bo Builder, Cy Coder et al. A study of things. Journal, 2020. doi:10.1/x.")
        self.assertEqual(refs[1], "2. Ed Else. Another paper. Journal, 2020. OpenAlex W2.")
        self.assertIn("Result: PASS", self.read("report-audit.md"))
        self.assertTrue((self.out / "ledger.json").is_file())


class CheckTests(ReportGateTestCase):
    def checks(self, draft: str, check: str) -> list[dict]:
        return [v for v in self.violations(draft) if v["check"] == check]

    def test_g1_missing_misordered_and_references_heading(self):
        draft = GOOD_DRAFT.replace("## Findings", "## Findings-tmp")
        draft = draft.replace("## New avenues", "## Findings").replace("## Findings-tmp", "## New avenues")
        draft = draft.replace("## Audit\n\nThree claims checked.\n", "## References\n\n1. x\n\n## Extra\n\nmore [@q1-p1#0]\n")
        details = [v["detail"] for v in self.checks(draft, "G1")]
        self.assertTrue(any("## Findings is out of order" in d for d in details), details)
        self.assertTrue(any(d == "missing ## Audit" for d in details), details)
        self.assertTrue(any("remove ## References" in d for d in details), details)
        self.assertTrue(any("unexpected ## Extra" in d for d in details), details)
        duplicated = GOOD_DRAFT + "\n## Audit\n\nagain [@q1-p1#0]\n"
        self.assertTrue(any("duplicate ## Audit" in v["detail"] for v in self.checks(duplicated, "G1")))

    def test_g2_orphan_and_malformed_keys(self):
        draft = GOOD_DRAFT.replace("Claim A holds [@q1-p1#0].", "Claim A holds [@q9-p9#0] and [@not a key].")
        g2 = self.checks(draft, "G2")
        self.assertEqual(len(g2), 2, g2)
        self.assertIn("[@q9-p9#0] is not a claim key", g2[0]["detail"] + g2[1]["detail"])
        self.assertIn("malformed citation '@not a key'", g2[0]["detail"] + g2[1]["detail"])
        self.assertEqual({v["line"] for v in g2}, {11})
        report, _, _ = report_gate.render_report(draft, report_gate.gate_common.build_ledger(self.out))
        self.assertIn("Claim A holds [?] and [?].", report)

    def test_g3_unverified_only_under_unverified_leads_and_forbidden_nowhere(self):
        draft = GOOD_DRAFT.replace("Claim A holds [@q1-p1#0].", "Claim A holds [@q1-p1#0; @q2-p1#0].")
        draft = draft.replace("- Claim B, unverified [@q2-p1#0].", "- Claim B, unverified [@q2-p1#0].\n- Claim C [@q3-p2#0].")
        g3 = self.checks(draft, "G3")
        self.assertEqual(len(g3), 2, g3)
        unverified = next(v for v in g3 if "q2-p1#0" in v["detail"])
        self.assertEqual(unverified["line"], 11)
        self.assertIn("cite it only under ### Unverified leads", unverified["detail"])
        forbidden = next(v for v in g3 if "q3-p2#0" in v["detail"])
        self.assertIn("is forbidden", forbidden["detail"])
        self.assertEqual(self.checks(GOOD_DRAFT, "G3"), [], "an unverified key under Unverified leads is fine")

    def test_g4_uncited_findings_block_and_uncited_avenue_subsection(self):
        draft = GOOD_DRAFT.replace(
            "- Evidence: a single study [@q1-p1#0; @q1-p1#0].",
            "- Evidence: a single study [@q1-p1#0; @q1-p1#0].\n- Counter-evidence: none found.\n\nA closing remark\nover two lines.",
        )
        draft = draft.replace("Promising because of [@q1-p1#0].", "Promising because of [@q1-p1#0].\n\n### Avenue two\n\nNo evidence yet.")
        g4 = self.checks(draft, "G4")
        details = [v["detail"] for v in g4]
        self.assertEqual(len(g4), 3, details)
        self.assertTrue(any("Counter-evidence: none found." in d for d in details))
        self.assertTrue(any("A closing remark over two lines." in d for d in details))
        self.assertTrue(any("### Avenue two under ## New avenues" in d for d in details))

    def test_g5_closed_question_without_a_cited_claim(self):
        helpers.write_pass(
            self.out,
            4,
            findings=[helpers.finding("q4-p4", [helpers.claim("Claim D")])],
            audit=helpers.audit(4, [helpers.audit_claim("q4-p4", 0, "supported")]),
        )
        fr = helpers.read_json(self.out, "frontier-v3.json")
        fr["questions"].append(helpers.question("q4", status="closed", supported_claims=["q4-p4#0"]))
        helpers.write_json(self.out, "frontier-v5.json", dict(fr, **{"pass": 5}))
        ledger = report_gate.gate_common.build_ledger(self.out)
        violations = report_gate.check_draft(GOOD_DRAFT, ledger, helpers.read_json(self.out, "frontier-v5.json"))
        self.assertEqual([v["check"] for v in violations], ["G5"], violations)
        self.assertIn("closed question q4 has supported claims (q4-p4#0) but none is cited", violations[0]["detail"])
        code, _ = self.gate(GOOD_DRAFT)
        self.assertEqual(code, 1, "the gate reads the highest-numbered frontier")


class RenderingTests(ReportGateTestCase):
    def test_two_keys_of_one_work_share_a_number(self):
        helpers.write_pass(
            self.out,
            4,
            findings=[helpers.finding("q4-p4", [helpers.claim("Claim D")])],
            audit=helpers.audit(4, [helpers.audit_claim("q4-p4", 0, "supported")]),
        )
        draft = GOOD_DRAFT.replace("Claim A holds [@q1-p1#0].", "Claim A holds [@q1-p1#0; @q4-p4#0], again [@q4-p4#0].")
        report, numbers, refs = report_gate.render_report(draft, report_gate.gate_common.build_ledger(self.out))
        self.assertIn("Claim A holds [1], again [1].", report)
        self.assertEqual(numbers["q1-p1#0"], numbers["q4-p4#0"])
        self.assertEqual(len([r for r in refs if "A study of things" in r]), 1)

    def test_unverified_key_without_work_renders_its_claimed_id(self):
        helpers.write_pass(
            self.out,
            4,
            findings=[helpers.finding("q5-p4", [helpers.claim("Claim E", work_id="W77")])],
            audit=helpers.audit(4, [helpers.audit_claim("q5-p4", 0, "cannot_verify", work={})]),
        )
        draft = GOOD_DRAFT.replace("- Claim B, unverified [@q2-p1#0].", "- Claim B, unverified [@q2-p1#0].\n- Claim E [@q5-p4#0].")
        code, out = self.gate(draft)
        self.assertEqual(code, 0, out)
        self.assertIn("- Claim E [unverified: W77].", self.read("report.md"))

    def test_failing_draft_still_writes_report_with_banner_and_audit(self):
        code, out = self.gate(GOOD_DRAFT.replace("[@q1-p1#0]", "[@q9-p1#0]", 1))
        self.assertEqual(code, 1)
        self.assertTrue(out.startswith("REPORT GATE: FAIL ("), out)
        self.assertIn("G2 line 5:", out)
        self.assertTrue(out.rstrip().endswith("Fix report-draft.md and keep every other passage unchanged."))
        self.assertTrue(self.read("report.md").startswith("> REPORT GATE FAILED:"))
        audit = self.read("report-audit.md")
        self.assertIn("Result: FAIL", audit)
        self.assertIn("| q9-p1#0 | orphan |", audit)

    def test_feedback_fits_gate_feedback_cap(self):
        bullets = "\n".join(f"- uncited bullet number {i} with some words" for i in range(200))
        draft = GOOD_DRAFT.replace("Claim A holds [@q1-p1#0].", "Claim A holds [@q1-p1#0].\n\n" + bullets)
        code, out = self.gate(draft)
        self.assertEqual(code, 1)
        self.assertIn("(200 violations)", out.splitlines()[0])
        self.assertLessEqual(len(out.rstrip("\n")), 3500)
        self.assertTrue(out.rstrip().endswith("Fix report-draft.md and keep every other passage unchanged."))
        self.assertIn("more lines)", out)

    def test_input_error_without_draft(self):
        code, out = helpers.run_main(report_gate, [self.out])
        self.assertEqual(code, 2)
        self.assertTrue(out.startswith("REPORT GATE: INPUT ERROR"), out)
        self.assertIn("report-draft.md", out)

    def test_input_error_without_audits_or_frontier(self):
        with tempfile.TemporaryDirectory() as other:
            out_dir = Path(other)
            (out_dir / "report-draft.md").write_text(GOOD_DRAFT, encoding="utf-8")
            code, out = helpers.run_main(report_gate, [out_dir])
            self.assertEqual(code, 2)
            self.assertIn("no audit-v*.json", out)

    def test_cli_subprocess(self):
        (self.out / "report-draft.md").write_text(GOOD_DRAFT, encoding="utf-8")
        result = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), str(self.out)], capture_output=True, text=True, encoding="utf-8"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(result.stdout.startswith("REPORT GATE: PASS"))
        (self.out / "report-draft.md").write_text(GOOD_DRAFT.replace("## Audit", "## Audits — x [@q9-p9#0]"), encoding="utf-8")
        failing = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), str(self.out)], capture_output=True, text=True, encoding="utf-8"
        )
        self.assertEqual(failing.returncode, 1, failing.stdout + failing.stderr)
        self.assertTrue(failing.stdout.startswith("REPORT GATE: FAIL ("), failing.stdout)
        # The heading's em dash reaches the feedback intact (stdout is UTF-8 even on a Windows pipe).
        self.assertIn("unexpected ## Audits — x", failing.stdout)


if __name__ == "__main__":
    unittest.main()
