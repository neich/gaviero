"""Tests for pass_gate.py (contract C3)."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import helpers  # noqa: E402
import pass_gate  # noqa: E402

SCRIPT = helpers.TOOLS_DIR / "pass_gate.py"


class PassGateTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.out = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def gate(self, n: int = 1) -> tuple[int, str]:
        return helpers.run_main(pass_gate, [self.out, n])

    def next_frontier(self, n: int = 1) -> dict:
        return helpers.read_json(self.out, f"frontier-v{n + 1}.json")

    def q(self, frontier: dict, qid: str) -> dict:
        return next(q for q in frontier["questions"] if q["id"] == qid)

    def metrics(self, n: int = 1) -> dict:
        return helpers.read_json(self.out, f"metrics-v{n}.json")

    def one_lane(self, verdicts, q1=None, others=(), children=(), novel_children=0, fidelity=(), n=1, extra=None):
        """Pass n with one lane on q1: one claim per verdict."""
        questions = [q1 or helpers.question("q1", status="active")] + list(others)
        aid = f"q1-p{n}"
        claims = [helpers.claim(f"claim {i}") for i in range(len(verdicts))]
        audited = [helpers.audit_claim(aid, i, v) for i, v in enumerate(verdicts)]
        for i, overrides in (extra or {}).items():
            audited[i].update(overrides)
        helpers.write_pass(
            self.out,
            n,
            frontier=helpers.frontier(n, questions),
            assignments=[aid],
            findings=[helpers.finding(aid, claims, children=children)],
            audit=helpers.audit(n, audited, fidelity=fidelity, novel_children=novel_children),
        )


class MergeTests(PassGateTestCase):
    def test_supported_claim_closes_question(self):
        self.one_lane(["supported"])
        self.gate()
        q1 = self.q(self.next_frontier(), "q1")
        self.assertEqual(q1["status"], "closed")
        self.assertEqual(q1["supported_claims"], ["q1-p1#0"])
        self.assertIn("q1", self.metrics()["merge"]["closed"])

    def test_substituted_lane_reopens_even_with_supported_claims(self):
        self.one_lane(["supported"], fidelity=[("q1-p1", "substituted")])
        self.gate()
        q1 = self.q(self.next_frontier(), "q1")
        self.assertEqual(q1["status"], "open")
        self.assertEqual(q1["supported_claims"], [])
        self.assertIn("q1", self.metrics()["merge"]["reopened"])

    def test_partial_keeps_open_and_lists_narrow(self):
        self.one_lane(["partial"], novel_children=1)
        code, out = self.gate()
        self.assertEqual(self.q(self.next_frontier(), "q1")["status"], "open")
        merge_line = next(line for line in out.splitlines() if line.startswith("merge:"))
        self.assertIn("narrow=[q1]", merge_line)
        self.assertEqual(code, 1)

    def test_second_empty_visit_marks_dead_and_appends_archive(self):
        self.one_lane(["not_supported"], q1=helpers.question("q1", status="active", visits=1), n=2)
        self.gate(2)
        q1 = self.q(self.next_frontier(2), "q1")
        self.assertEqual(q1["status"], "dead")
        self.assertNotIn("visits", q1)
        archive = (self.out / "archive.md").read_text(encoding="utf-8")
        self.assertIn("## Pass 2 (pass gate)", archive)
        self.assertIn("q1 (pass 2, pass gate)", archive)

    def test_first_empty_visit_stays_open(self):
        self.one_lane(["not_supported"])
        self.gate()
        q1 = self.q(self.next_frontier(), "q1")
        self.assertEqual(q1["status"], "open")
        self.assertEqual(q1["visits"], 1)

    def test_closed_question_status_never_changes(self):
        closed = helpers.question("q1", status="closed", supported_claims=["q1-p0#0"])
        helpers.write_pass(
            self.out,
            1,
            frontier=helpers.frontier(1, [closed]),
            assignments=["q1-p1"],
            findings=[helpers.finding("q1-p1", [], outcome="nothing_found")],
            audit=helpers.audit(1, []),
        )
        self.gate()
        q1 = self.q(self.next_frontier(), "q1")
        self.assertEqual(q1["status"], "closed")
        self.assertEqual(q1["supported_claims"], ["q1-p0#0"])
        self.assertEqual(q1["kind"], "mechanism")

    def test_children_added_with_parent_depth_and_next_id(self):
        self.one_lane(
            ["partial"],
            q1=helpers.question("q1", status="active", depth=1),
            others=[helpers.question("q2"), helpers.question("q3")],
            children=[helpers.child("Does the effect reverse in the cold?", kind="contrarian", why="a dispute")],
        )
        self.gate()
        fr = self.next_frontier()
        q4 = self.q(fr, "q4")
        self.assertEqual(q4["parent"], "q1")
        self.assertEqual(q4["depth"], 2)
        self.assertEqual(q4["operator"], "child")
        self.assertEqual(q4["status"], "open")
        self.assertEqual(q4["kind"], "contrarian")
        self.assertEqual(q4["scores"], {"relevance": 0, "gain": 0, "novelty": 0, "tractability": 0})
        self.assertEqual(q4["priority"], -2)
        self.assertEqual(q4["why"], "a dispute")
        self.assertEqual(fr["metrics"]["created_total"], 4)
        self.assertEqual(self.metrics()["merge"]["children_added"], ["q4"])

    def test_children_skipped_for_depth_cap_duplicate_and_unknown_parent(self):
        questions = [
            helpers.question("q1", status="active", depth=3),
            helpers.question("q2", status="active"),
            helpers.question("q3", question="Is it the heat?"),
        ]
        helpers.write_pass(
            self.out,
            1,
            frontier=helpers.frontier(1, questions),
            assignments=["q1-p1", "q2-p1", "q9-p1"],
            findings=[
                helpers.finding("q1-p1", [], children=[helpers.child("Too deep?")]),
                helpers.finding("q2-p1", [], children=[helpers.child("is it THE heat")]),
                helpers.finding("q9-p1", [], children=[helpers.child("Orphan?")]),
            ],
            audit=helpers.audit(1, []),
        )
        self.gate()
        merge = self.metrics()["merge"]
        reasons = {s["question"]: s["reason"] for s in merge["children_skipped"]}
        self.assertEqual(reasons, {"Too deep?": "depth", "is it THE heat": "duplicate", "Orphan?": "unknown_parent"})
        self.assertEqual(merge["unknown_assignments"], ["q9-p1"])

        with tempfile.TemporaryDirectory() as other:
            out = Path(other)
            helpers.write_pass(
                out,
                1,
                frontier=helpers.frontier(1, [helpers.question("q1", status="active")], created_total=40),
                assignments=["q1-p1"],
                findings=[helpers.finding("q1-p1", [], children=[helpers.child("One more?")])],
                audit=helpers.audit(1, []),
            )
            helpers.run_main(pass_gate, [out, 1])
            skipped = helpers.read_json(out, "metrics-v1.json")["merge"]["children_skipped"]
            self.assertEqual(skipped, [{"question": "One more?", "reason": "cap"}])

    def test_retracted_claim_dropped_and_question_reopened(self):
        questions = [
            helpers.question("q1", status="closed", supported_claims=["q5-p1#0"]),
            helpers.question("q5", status="active"),
        ]
        helpers.write_pass(
            self.out,
            1,
            frontier=helpers.frontier(1, questions),
            assignments=["q5-p1"],
            findings=[helpers.finding("q5-p1", [helpers.claim("retracted finding")])],
            audit=helpers.audit(1, [helpers.audit_claim("q5-p1", 0, "supported", retracted=True)]),
        )
        self.gate()
        q1 = self.q(self.next_frontier(), "q1")
        self.assertEqual(q1["status"], "open")
        self.assertEqual(q1["supported_claims"], [])
        merge = self.metrics()["merge"]
        self.assertEqual(merge["dropped_claims"], ["q5-p1#0"])
        self.assertIn("q1", merge["reopened"])
        archive = (self.out / "archive.md").read_text(encoding="utf-8")
        self.assertIn("claim q5-p1#0 dropped (pass 1, pass gate): retracted; retracted finding", archive)

    def test_lost_ids_restored_from_previous_frontier(self):
        helpers.write_pass(
            self.out,
            1,
            frontier=helpers.frontier(1, [helpers.question("q1"), helpers.question("q2"), helpers.question("q3")]),
        )
        helpers.write_pass(
            self.out,
            2,
            frontier=helpers.frontier(2, [helpers.question("q1"), helpers.question("q3")]),
            assignments=[],
            audit=helpers.audit(2, []),
        )
        self.gate(2)
        fr = self.next_frontier(2)
        self.assertEqual([q["id"] for q in fr["questions"]], ["q1", "q3", "q2"])
        self.assertEqual(self.metrics(2)["merge"]["restored"], ["q2"])

    def test_priority_recomputed_compaction_and_truncation(self):
        long_question = "Why " + "x" * 296
        q1 = helpers.question(
            "q1",
            depth=1,
            priority=99,
            question=long_question,
            why="w" * 400,
            scores={"relevance": 3, "gain": 2, "novelty": 1, "tractability": 0},
            extra_field="dropped",
        )
        q2 = helpers.question("q2", status="closed", supported_claims=["k#0"])
        q3 = helpers.question("q3", status="parked")
        leading = [f"claim {i}" for i in range(6)] + ["L" * 250]
        leading[0] = "L" * 250
        helpers.write_pass(
            self.out,
            1,
            frontier=helpers.frontier(1, [q1, q2, q3], leading_claims=leading),
            assignments=[],
            audit=helpers.audit(1, []),
        )
        self.gate()
        fr = self.next_frontier()
        self.assertEqual(list(fr), ["pass", "collection", "questions", "leading_claims", "metrics"])
        out1 = self.q(fr, "q1")
        self.assertEqual(out1["priority"], 5)
        self.assertEqual(len(out1["question"]), 250)
        self.assertTrue(out1["question"].endswith("…"))
        self.assertEqual(len(out1["why"]), 300)
        self.assertNotIn("extra_field", out1)
        self.assertEqual(
            set(self.q(fr, "q2")),
            {"id", "question", "kind", "status", "operator", "parent", "depth", "supported_claims"},
        )
        self.assertEqual(
            set(self.q(fr, "q3")),
            {"id", "question", "kind", "status", "operator", "parent", "depth", "supported_claims", "why"},
        )
        self.assertEqual(len(fr["leading_claims"]), 5)
        self.assertEqual(len(fr["leading_claims"][0]), 200)
        self.assertEqual(fr["metrics"], {"open": 1, "closed": 1, "dead": 0, "parked": 1, "created_total": 3})
        self.assertIn("q1", self.metrics()["merge"]["truncated"])
        self.assertIn("leading_claims", self.metrics()["merge"]["truncated"])


class StopRuleTests(PassGateTestCase):
    def test_stop_s0_empty_assignments(self):
        helpers.write_pass(
            self.out, 1, frontier=helpers.frontier(1, [helpers.question("q1")]), assignments=[], audit=helpers.audit(1, [], novel_children=3)
        )
        code, out = self.gate()
        self.assertEqual(code, 0)
        self.assertTrue(out.splitlines()[0].startswith("PASS GATE v1: STOP"), out)
        self.assertIn("S0", out.splitlines()[0])

    def test_stop_s1_exhausted(self):
        low = {"relevance": 1, "gain": 1, "novelty": 1, "tractability": 1}
        self.one_lane(["partial"], q1=helpers.question("q1", status="active", scores=low), novel_children=0)
        code, out = self.gate()
        self.assertEqual(code, 0, out)
        self.assertIn("S1", out.splitlines()[0])
        self.assertTrue(self.metrics()["rules"]["S1"])

    def test_stop_s1_needs_no_high_priority_open_question(self):
        self.one_lane(["partial"], novel_children=0)  # q1 stays open with priority 8
        code, _ = self.gate()
        self.assertEqual(code, 1)
        self.assertFalse(self.metrics()["rules"]["S1"])

    def test_stop_s2_needs_two_saturated_passes(self):
        self.one_lane(["supported"], others=[helpers.question("q2")], novel_children=1)
        code1, _ = self.gate(1)
        self.assertEqual(code1, 1, "S2 needs two passes")
        helpers.write_pass(
            self.out,
            2,
            assignments=["q2-p2"],
            findings=[helpers.finding("q2-p2", [helpers.claim("weak")])],
            audit=helpers.audit(2, [helpers.audit_claim("q2-p2", 0, "partial")], novel_children=1),
        )
        code2, out2 = self.gate(2)
        self.assertEqual(code2, 0, out2)
        self.assertIn("S2", out2.splitlines()[0])
        self.assertFalse(self.metrics(2)["rules"]["S1"])

    def test_stop_s3_core_questions_answered(self):
        relevant_one = {"relevance": 1, "gain": 3, "novelty": 3, "tractability": 3}
        self.one_lane(
            ["supported"],
            q1=helpers.question("q1", status="active", kind="core"),
            others=[helpers.question("q2", scores=relevant_one)],
            novel_children=2,
        )
        code, out = self.gate()
        self.assertEqual(code, 0, out)
        self.assertIn("S3", out.splitlines()[0])

    def test_cannot_verify_blocks_s1_and_s2_but_not_s3(self):
        low = {"relevance": 1, "gain": 1, "novelty": 1, "tractability": 1}
        # S1 holds but two cannot_verify claims block it.
        self.one_lane(["cannot_verify", "cannot_verify"], q1=helpers.question("q1", status="active", scores=low))
        code, out = self.gate()
        self.assertEqual(code, 1, out)
        rules = self.metrics()["rules"]
        self.assertTrue(rules["S1"])
        self.assertTrue(rules["blocked"])

        # S2 holds on pass 2 but is blocked the same way.
        with tempfile.TemporaryDirectory() as other:
            out_dir = Path(other)
            helpers.write_pass(
                out_dir,
                1,
                frontier=helpers.frontier(1, [helpers.question("q1", status="active"), helpers.question("q2")]),
                assignments=["q1-p1"],
                findings=[helpers.finding("q1-p1", [helpers.claim("c")])],
                audit=helpers.audit(1, [helpers.audit_claim("q1-p1", 0, "partial")], novel_children=1),
            )
            helpers.run_main(pass_gate, [out_dir, 1])
            helpers.write_pass(
                out_dir,
                2,
                assignments=["q2-p2"],
                findings=[helpers.finding("q2-p2", [helpers.claim("a"), helpers.claim("b")])],
                audit=helpers.audit(
                    2,
                    [helpers.audit_claim("q2-p2", 0, "cannot_verify"), helpers.audit_claim("q2-p2", 1, "cannot_verify")],
                    novel_children=1,
                ),
            )
            code2, out2 = helpers.run_main(pass_gate, [out_dir, 2])
            self.assertEqual(code2, 1, out2)
            rules2 = helpers.read_json(out_dir, "metrics-v2.json")["rules"]
            self.assertTrue(rules2["S2"])
            self.assertTrue(rules2["blocked"])

        # S3 is not blocked.
        with tempfile.TemporaryDirectory() as other:
            out_dir = Path(other)
            aid = "q1-p1"
            helpers.write_pass(
                out_dir,
                1,
                frontier=helpers.frontier(1, [helpers.question("q1", status="active", kind="core")]),
                assignments=[aid],
                findings=[helpers.finding(aid, [helpers.claim("a"), helpers.claim("b"), helpers.claim("c")])],
                audit=helpers.audit(
                    1,
                    [
                        helpers.audit_claim(aid, 0, "supported"),
                        helpers.audit_claim(aid, 1, "cannot_verify"),
                        helpers.audit_claim(aid, 2, "cannot_verify"),
                    ],
                ),
            )
            code3, out3 = helpers.run_main(pass_gate, [out_dir, 1])
            self.assertEqual(code3, 0, out3)
            self.assertIn("S3", out3.splitlines()[0])

    def test_missing_novel_children_never_fires_s1_or_s2(self):
        low = {"relevance": 1, "gain": 1, "novelty": 1, "tractability": 1}
        self.one_lane(["partial"], q1=helpers.question("q1", status="active", scores=low), novel_children=None)
        code, out = self.gate()
        self.assertEqual(code, 1, out)
        self.assertIn("novel_children=missing", out)
        rules = self.metrics()["rules"]
        self.assertFalse(rules["S1"])
        self.assertFalse(rules["S2"])
        self.assertIsNone(self.metrics()["novel_children"])


class OutputTests(PassGateTestCase):
    def test_continue_writes_next_frontier_metrics_and_ledger(self):
        self.one_lane(["partial"], novel_children=1)
        code, out = self.gate()
        self.assertEqual(code, 1)
        self.assertTrue(out.splitlines()[0].startswith("PASS GATE v1: CONTINUE"), out)
        fr = self.next_frontier()
        self.assertEqual(fr["pass"], 2)
        self.assertEqual(sorted(fr), sorted(["pass", "collection", "questions", "leading_claims", "metrics"]))
        self.assertEqual(self.metrics()["decision"], "continue")
        self.assertTrue((self.out / "ledger.json").is_file())
        self.assertIn("q1-p1#0", helpers.read_json(self.out, "ledger.json")["claims"])
        self.assertIn("not an error", out)

    def test_stop_still_writes_next_frontier(self):
        helpers.write_pass(
            self.out, 1, frontier=helpers.frontier(1, [helpers.question("q1")]), assignments=[], audit=helpers.audit(1, [])
        )
        code, _ = self.gate()
        self.assertEqual(code, 0)
        self.assertEqual(self.next_frontier()["pass"], 2)
        self.assertEqual(self.metrics()["decision"], "stop")

    def test_verifier_metric_mismatch_is_reported(self):
        self.one_lane(["supported"], others=[helpers.question("q2")], novel_children=1)
        audit = helpers.read_json(self.out, "audit-v1.json")
        audit["metrics"]["supported"] = 5
        helpers.write_json(self.out, "audit-v1.json", audit)
        _, out = self.gate()
        self.assertIn("mismatch: verifier supported=5 vs recount 1", out)
        self.assertEqual(self.metrics()["mismatches"], ["verifier supported=5 vs recount 1"])

    def test_input_error_exit_2(self):
        helpers.write_pass(self.out, 1, frontier=helpers.frontier(1, [helpers.question("q1")]), assignments=[])
        code, out = self.gate()
        self.assertEqual(code, 2)
        self.assertTrue(out.splitlines()[0].startswith("PASS GATE v1: INPUT ERROR"), out)
        self.assertIn("audit-v1.json", out)
        self.assertIn("error", self.metrics())

    def test_report_fits_gate_feedback_cap(self):
        questions = [helpers.question(f"q{i}", status="active" if i <= 30 else "open") for i in range(1, 61)]
        findings = [
            helpers.finding(f"q{i}-p1", [helpers.claim("c")], children=[helpers.child(f"Child question number {i}?")])
            for i in range(1, 31)
        ]
        audited = [helpers.audit_claim(f"q{i}-p1", 0, "partial") for i in range(1, 31)]
        helpers.write_pass(
            self.out,
            1,
            frontier=helpers.frontier(1, questions),
            assignments=[f"q{i}-p1" for i in range(1, 31)],
            findings=findings,
            audit=helpers.audit(1, audited, novel_children=30),
        )
        _, out = self.gate()
        self.assertLessEqual(len(out.rstrip("\n")), 3500)
        self.assertIn("narrow=[q1,q2,", out)

    def test_cli_subprocess(self):
        helpers.write_pass(
            self.out, 1, frontier=helpers.frontier(1, [helpers.question("q1")]), assignments=[], audit=helpers.audit(1, [])
        )
        result = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), str(self.out), "1"], capture_output=True, text=True, encoding="utf-8"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.out / "frontier-v2.json").is_file())
        self.assertTrue(result.stdout.startswith("PASS GATE v1: STOP"))

        bad = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), str(self.out), "zero"], capture_output=True, text=True, encoding="utf-8"
        )
        self.assertEqual(bad.returncode, 2)


if __name__ == "__main__":
    unittest.main()
