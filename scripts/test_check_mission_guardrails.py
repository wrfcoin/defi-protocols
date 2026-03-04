from __future__ import annotations

import sys
from pathlib import Path
import tempfile
import textwrap
import unittest

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import check_mission_guardrails as cmg


class MissionGuardrailValidatorTests(unittest.TestCase):
    def test_validate_release_gate_reports_unchecked_required_guardrail(self) -> None:
        text = textwrap.dedent(
            """\
            # Gate
            - [x] privacy.financial_minimization
            - [ ] governance.equal_liability
            """
        )
        errors = cmg.validate_release_gate(
            Path("gate.md"),
            text,
            {"privacy.financial_minimization", "governance.equal_liability"},
        )
        self.assertEqual(len(errors), 1)
        self.assertIn("governance.equal_liability", errors[0])

    def test_validate_proposal_ready_for_merge_requires_checked_guardrails(self) -> None:
        proposal = textwrap.dedent(
            """\
            ---
            proposal_id: "defi-20260304-test"
            status: "ready-for-merge"
            ---
            # Proposal
            ## Mission Guardrails Checklist
            - [x] privacy.financial_minimization
            - [ ] governance.equal_liability

            ## Privacy Impact
            ok
            ## Environmental Utility Impact
            ok
            ## Equal-Liability Assessment
            ok
            ## Abuse Controls
            ok
            ## Rollback Plan
            ok
            """
        )
        errors = cmg.validate_proposal(
            Path("proposal.md"),
            proposal,
            {"privacy.financial_minimization", "governance.equal_liability"},
        )
        self.assertEqual(len(errors), 1)
        self.assertIn("governance.equal_liability", errors[0])

    def test_validate_proposal_draft_does_not_block_unchecked_items(self) -> None:
        proposal = textwrap.dedent(
            """\
            ---
            proposal_id: "defi-20260304-test"
            status: "draft"
            ---
            # Proposal
            ## Mission Guardrails Checklist
            - [ ] privacy.financial_minimization
            - [ ] governance.equal_liability
            """
        )
        errors = cmg.validate_proposal(
            Path("proposal.md"),
            proposal,
            {"privacy.financial_minimization", "governance.equal_liability"},
        )
        self.assertEqual(errors, [])

    def test_collect_errors_fails_when_required_section_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            (root / "docs/governance").mkdir(parents=True)
            (root / "docs/proposals").mkdir(parents=True)

            (root / "docs/governance/mission-guardrails.toml").write_text(
                textwrap.dedent(
                    """\
                    [[guardrail]]
                    id = "privacy.financial_minimization"
                    required = true
                    applies_to = ["merge", "release"]
                    """
                ),
                encoding="utf-8",
            )
            (root / "docs/governance/mission-guardrail-status.md").write_text(
                "- [x] privacy.financial_minimization\n", encoding="utf-8"
            )
            (root / "docs/proposals/proposal.md").write_text(
                textwrap.dedent(
                    """\
                    ---
                    status: "ready-for-merge"
                    ---
                    # Proposal
                    ## Mission Guardrails Checklist
                    - [x] privacy.financial_minimization
                    """
                ),
                encoding="utf-8",
            )

            errors = cmg.collect_errors(
                guardrails_path=root / "docs/governance/mission-guardrails.toml",
                release_gate_path=root / "docs/governance/mission-guardrail-status.md",
                proposals_dir=root / "docs/proposals",
            )
            self.assertTrue(errors)
            self.assertTrue(
                any("## Privacy Impact" in error for error in errors),
                f"missing section error not found: {errors}",
            )


if __name__ == "__main__":
    unittest.main()
