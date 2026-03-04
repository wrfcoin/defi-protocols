#!/usr/bin/env python3
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys
import tomllib

REQUIRED_PROPOSAL_STATUSES = {"ready-for-merge", "ready-for-release", "approved"}
PROPOSAL_REQUIRED_SECTIONS = (
    "## Mission Guardrails Checklist",
    "## Privacy Impact",
    "## Environmental Utility Impact",
    "## Equal-Liability Assessment",
    "## Abuse Controls",
    "## Rollback Plan",
)
CHECKBOX_RE = re.compile(r"^\s*-\s*\[(?P<mark>[xX ])\]\s*(?P<id>[a-z0-9][a-z0-9._-]*)\b")


@dataclass(frozen=True)
class Guardrail:
    id: str
    title: str
    required: bool
    applies_to: frozenset[str]


def normalize_newlines(text: str) -> str:
    return text.replace("\r\n", "\n").replace("\r", "\n")


def parse_frontmatter(text: str) -> dict[str, str]:
    normalized = normalize_newlines(text)
    if not normalized.startswith("---\n"):
        return {}
    end = normalized.find("\n---\n", 4)
    if end == -1:
        return {}
    block = normalized[4:end]
    parsed: dict[str, str] = {}
    for line in block.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or ":" not in stripped:
            continue
        key, value = stripped.split(":", 1)
        parsed[key.strip().lower()] = value.strip().strip("\"'")
    return parsed


def extract_checkbox_states(text: str) -> dict[str, bool]:
    states: dict[str, bool] = {}
    for line in normalize_newlines(text).splitlines():
        match = CHECKBOX_RE.match(line)
        if not match:
            continue
        states[match.group("id")] = match.group("mark").lower() == "x"
    return states


def load_guardrails(path: Path) -> list[Guardrail]:
    payload = tomllib.loads(path.read_text(encoding="utf-8"))
    rows = payload.get("guardrail", [])
    guardrails: list[Guardrail] = []
    for row in rows:
        if "id" not in row:
            raise ValueError(f"{path}: guardrail missing 'id'")
        guardrails.append(
            Guardrail(
                id=str(row["id"]),
                title=str(row.get("title", row["id"])),
                required=bool(row.get("required", False)),
                applies_to=frozenset(str(v) for v in row.get("applies_to", [])),
            )
        )
    return guardrails


def required_guardrail_ids(
    guardrails: list[Guardrail], contexts: set[str] | None = None
) -> set[str]:
    target_contexts = contexts or {"merge", "release"}
    return {
        g.id
        for g in guardrails
        if g.required and (not g.applies_to or not target_contexts.isdisjoint(g.applies_to))
    }


def validate_release_gate(path: Path, text: str, required_ids: set[str]) -> list[str]:
    errors: list[str] = []
    checkbox_states = extract_checkbox_states(text)
    for guardrail_id in sorted(required_ids):
        if guardrail_id not in checkbox_states:
            errors.append(f"{path}: missing required guardrail checkbox '{guardrail_id}'")
            continue
        if not checkbox_states[guardrail_id]:
            errors.append(f"{path}: required guardrail '{guardrail_id}' is unchecked")
    return errors


def validate_proposal(path: Path, text: str, required_ids: set[str]) -> list[str]:
    errors: list[str] = []
    metadata = parse_frontmatter(text)
    status = metadata.get("status", "").strip().lower()
    if status not in REQUIRED_PROPOSAL_STATUSES:
        return errors

    for section in PROPOSAL_REQUIRED_SECTIONS:
        if section.lower() not in text.lower():
            errors.append(f"{path}: missing required section '{section}' for status '{status}'")

    checkbox_states = extract_checkbox_states(text)
    for guardrail_id in sorted(required_ids):
        if guardrail_id not in checkbox_states:
            errors.append(
                f"{path}: status '{status}' requires guardrail '{guardrail_id}' checkbox entry"
            )
            continue
        if not checkbox_states[guardrail_id]:
            errors.append(
                f"{path}: status '{status}' requires guardrail '{guardrail_id}' to be checked"
            )
    return errors


def discover_proposals(proposals_dir: Path) -> list[Path]:
    if not proposals_dir.exists():
        return []
    files = sorted(proposals_dir.glob("*.md"))
    return [
        path
        for path in files
        if "template" not in path.name.lower() and path.name.lower() != "readme.md"
    ]


def collect_errors(
    *,
    guardrails_path: Path,
    release_gate_path: Path,
    proposals_dir: Path,
) -> list[str]:
    guardrails = load_guardrails(guardrails_path)
    required_ids = required_guardrail_ids(guardrails)
    errors: list[str] = []

    if not release_gate_path.exists():
        errors.append(f"{release_gate_path}: release gate file is missing")
        return errors

    release_text = release_gate_path.read_text(encoding="utf-8")
    errors.extend(validate_release_gate(release_gate_path, release_text, required_ids))

    for proposal_path in discover_proposals(proposals_dir):
        proposal_text = proposal_path.read_text(encoding="utf-8")
        errors.extend(validate_proposal(proposal_path, proposal_text, required_ids))

    return errors


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Validate DeFi mission-alignment guardrails for merge/release gates."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path("."),
        help="Repository root containing docs/ and scripts/",
    )
    parser.add_argument(
        "--guardrails-file",
        type=Path,
        default=Path("docs/governance/mission-guardrails.toml"),
        help="Path to mission guardrail specification file (relative to --repo-root).",
    )
    parser.add_argument(
        "--release-gate-file",
        type=Path,
        default=Path("docs/governance/mission-guardrail-status.md"),
        help="Path to release guardrail status file (relative to --repo-root).",
    )
    parser.add_argument(
        "--proposals-dir",
        type=Path,
        default=Path("docs/proposals"),
        help="Path to DeFi proposal documents directory (relative to --repo-root).",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    repo_root = args.repo_root.resolve()
    guardrails_path = (repo_root / args.guardrails_file).resolve()
    release_gate_path = (repo_root / args.release_gate_file).resolve()
    proposals_dir = (repo_root / args.proposals_dir).resolve()

    try:
        errors = collect_errors(
            guardrails_path=guardrails_path,
            release_gate_path=release_gate_path,
            proposals_dir=proposals_dir,
        )
    except (ValueError, tomllib.TOMLDecodeError) as exc:
        print(f"Guardrail validation configuration error: {exc}", file=sys.stderr)
        return 2

    if errors:
        print("Mission guardrail validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    print("Mission guardrail validation passed.")
    print(f"Checked: {guardrails_path}")
    print(f"Checked: {release_gate_path}")
    print(f"Checked proposals in: {proposals_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
