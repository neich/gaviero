"""Tests for gate_common.py (contract C2)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import helpers  # noqa: E402
import gate_common  # noqa: E402


class WorkKeyTests(unittest.TestCase):
    def test_work_key_prefers_openalex_then_doi_then_arxiv_then_title(self):
        full = {"openalex": "W7", "doi": "10.1/A", "arxiv": "2101.00001", "title": "T"}
        self.assertEqual(gate_common.work_key(full), "W7")
        self.assertEqual(gate_common.work_key(dict(full, openalex="")), "doi:10.1/a")
        self.assertEqual(gate_common.work_key(dict(full, openalex="", doi="")), "arxiv:2101.00001")
        self.assertEqual(
            gate_common.work_key({"openalex": "", "doi": "", "arxiv": "", "title": "A Study: of Things!"}),
            "title:a study of things",
        )
        self.assertIsNone(gate_common.work_key({}))
        self.assertIsNone(gate_common.work_key(None))
        # An OpenAlex value that is not a work id falls through to the DOI.
        self.assertEqual(gate_common.work_key({"openalex": "A123", "doi": "10.2/x"}), "doi:10.2/x")

    def test_work_key_strips_url_prefixes(self):
        self.assertEqual(gate_common.work_key({"openalex": "https://openalex.org/W12"}), "W12")
        self.assertEqual(gate_common.work_key({"doi": "https://doi.org/10.1/AbC"}), "doi:10.1/abc")


class TitleTests(unittest.TestCase):
    def test_title_ratio_threshold(self):
        self.assertEqual(gate_common.title_ratio("A study of things", "A Study of Things."), 1.0)
        ratio = gate_common.title_ratio(
            "Attention is all you need", "Deep residual learning for image recognition"
        )
        self.assertLess(ratio, gate_common.TITLE_MISMATCH_THRESHOLD)
        self.assertIsNone(gate_common.title_ratio("", "Anything"))
        self.assertIsNone(gate_common.title_ratio("Anything", "  --  "))
        self.assertEqual(gate_common.TITLE_MISMATCH_THRESHOLD, 0.70)

    def test_question_id_of(self):
        self.assertEqual(gate_common.question_id_of("q7-p2"), "q7")
        self.assertEqual(gate_common.question_id_of("q7"), "q7")
        self.assertEqual(gate_common.question_id_of("q-7-p12"), "q-7")


class LedgerTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.out = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def test_ledger_citable_classes(self):
        cases = [
            # (verdict, audit overrides, expected citable, expected flag or None)
            ("supported", {}, "supported", None),
            ("partial", {}, "partial", None),
            ("cannot_verify", {}, "unverified", None),
            ("not_supported", {}, "forbidden", None),
            ("supported", {"retracted": True}, "forbidden", "retracted"),
            ("supported", {"resolved": False}, "forbidden", "unresolvable"),
            ("supported", {"work": helpers.work(openalex="W6", title="Deep residual learning for image recognition")}, "unverified", "title_mismatch"),
            ("supported", {"work": {}}, "unverified", "no_work"),
        ]
        claims = [helpers.claim(f"claim {i}") for i in range(len(cases) + 1)]
        helpers.write_pass(self.out, 1, findings=[helpers.finding("q1-p1", claims)])
        audited = []
        for i, (verdict, overrides, _, _) in enumerate(cases):
            c = helpers.audit_claim("q1-p1", i, verdict)
            c.update(overrides)
            audited.append(c)
        helpers.write_pass(self.out, 1, audit=helpers.audit(1, audited))

        ledger = gate_common.build_ledger(self.out)
        for i, (verdict, _, citable, flag) in enumerate(cases):
            entry = ledger["claims"][f"q1-p1#{i}"]
            self.assertEqual(entry["citable"], citable, f"case {i} ({verdict})")
            if flag:
                self.assertIn(flag, entry["flags"], f"case {i}")
        unaudited = ledger["claims"][f"q1-p1#{len(cases)}"]
        self.assertEqual(unaudited["citable"], "unverified")
        self.assertIn("no_audit", unaudited["flags"])
        self.assertEqual(unaudited["question_id"], "q1")
        self.assertEqual(ledger["sources"], ["audit-v1.json", "q1-p1-finding-v1.json"])

    def test_ledger_later_audit_wins(self):
        helpers.write_pass(self.out, 1, findings=[helpers.finding("q1-p1", [helpers.claim("c")])])
        helpers.write_pass(self.out, 1, audit=helpers.audit(1, [helpers.audit_claim("q1-p1", 0, "cannot_verify")]))
        helpers.write_pass(self.out, 2, audit=helpers.audit(2, [helpers.audit_claim("q1-p1", 0, "supported")]))
        entry = gate_common.build_ledger(self.out)["claims"]["q1-p1#0"]
        self.assertEqual(entry["verdict"], "supported")
        self.assertEqual(entry["citable"], "supported")
        self.assertEqual(entry["pass"], 1)

    def test_ledger_merges_works_by_key(self):
        helpers.write_pass(
            self.out,
            1,
            findings=[
                helpers.finding("q1-p1", [helpers.claim("a")]),
                helpers.finding("q2-p1", [helpers.claim("b")]),
            ],
            audit=helpers.audit(
                1,
                [
                    helpers.audit_claim("q1-p1", 0, "supported", work=helpers.work(venue="")),
                    helpers.audit_claim("q2-p1", 0, "partial", work=helpers.work(doi="10.9/z")),
                ],
            ),
        )
        ledger = gate_common.build_ledger(self.out)
        self.assertEqual(list(ledger["works"]), ["W1"])
        w = ledger["works"]["W1"]
        self.assertEqual(w["claims"], ["q1-p1#0", "q2-p1#0"])
        self.assertEqual(w["doi"], "10.9/z", "first non-empty value per field")
        self.assertEqual(w["venue"], "Journal")
        self.assertEqual(ledger["claims"]["q2-p1#0"]["work_key"], "W1")

    def test_ledger_audit_only_claim_is_flagged_no_finding(self):
        helpers.write_pass(self.out, 1, audit=helpers.audit(1, [helpers.audit_claim("q4-p1", 2, "supported")]))
        entry = gate_common.build_ledger(self.out)["claims"]["q4-p1#2"]
        self.assertIn("no_finding", entry["flags"])
        self.assertEqual(entry["question_id"], "q4")

    def test_write_json_is_utf8_and_atomic(self):
        target = self.out / "x.json"
        gate_common.write_json(target, {"t": "Pérez — 東京"})
        self.assertEqual(gate_common.load_json(target), {"t": "Pérez — 東京"})
        self.assertIn("Pérez", target.read_text(encoding="utf-8"))
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), ["x.json"])

    def test_load_json_errors_name_the_file(self):
        with self.assertRaises(gate_common.GateInputError) as missing:
            gate_common.load_json(self.out / "nope.json")
        self.assertTrue(str(missing.exception).startswith("nope.json:"))
        (self.out / "bad.json").write_text("{", encoding="utf-8")
        with self.assertRaises(gate_common.GateInputError) as bad:
            gate_common.load_json(self.out / "bad.json")
        self.assertTrue(str(bad.exception).startswith("bad.json:"))


class CapTextTests(unittest.TestCase):
    def test_cap_text_keeps_whole_lines(self):
        lines = [f"line {i} " + "x" * 50 for i in range(200)]
        text = gate_common.cap_text(lines, limit=500)
        self.assertLessEqual(len(text), 500)
        self.assertTrue(text.endswith("more lines)"))
        for line in text.split("\n")[:-1]:
            self.assertIn(line, lines)
        self.assertEqual(gate_common.cap_text(["a", "b"]), "a\nb")


if __name__ == "__main__":
    unittest.main()
