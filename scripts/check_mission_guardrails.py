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


@dataclass(frozen=True)
class DeprecationGatePolicy:
    enabled: bool
    allowed_statuses: frozenset[str]
    release_block_statuses: frozenset[str]
    require_remediation_issue_for_non_active: bool


@dataclass(frozen=True)
class ModuleStatus:
    module_id: str
    status: str
    failing_guardrails: frozenset[str]
    remediation_issue: str
    remediation_notes: str


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


def _load_payload(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _parse_guardrails(payload: dict, path: Path) -> list[Guardrail]:
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


def _parse_deprecation_gate_policy(payload: dict, path: Path) -> DeprecationGatePolicy:
    raw = payload.get("deprecation_gate", {})
    allowed = frozenset(str(v).strip().lower() for v in raw.get("allowed_statuses", []))
    if not allowed:
        allowed = frozenset({"active", "deprecated", "remediation"})

    release_block = frozenset(
        str(v).strip().lower() for v in raw.get("release_block_statuses", [])
    )

    if not release_block.issubset(allowed):
        raise ValueError(
            f"{path}: deprecation_gate.release_block_statuses must be a subset of allowed_statuses"
        )

    return DeprecationGatePolicy(
        enabled=bool(raw.get("enabled", False)),
        allowed_statuses=allowed,
        release_block_statuses=release_block,
        require_remediation_issue_for_non_active=bool(
            raw.get("require_remediation_issue_for_non_active", True)
        ),
    )


def load_guardrails(path: Path) -> list[Guardrail]:
    payload = _load_payload(path)
    return _parse_guardrails(payload, path)


def load_deprecation_gate_policy(path: Path) -> DeprecationGatePolicy:
    payload = _load_payload(path)
    return _parse_deprecation_gate_policy(payload, path)


def required_guardrail_ids(
    guardrails: list[Guardrail], contexts: set[str] | None = None
) -> set[str]:
    target_contexts = contexts or {"merge", "release"}
    return {
        g.id
        for g in guardrails
        if g.required and (not g.applies_to or not target_contexts.isdisjoint(g.applies_to))
    }


def load_module_statuses(path: Path) -> list[ModuleStatus]:
    payload = _load_payload(path)
    rows = payload.get("module", [])
    statuses: list[ModuleStatus] = []
    for row in rows:
        module_id = str(row.get("id", "")).strip()
        if not module_id:
            raise ValueError(f"{path}: module row missing non-empty 'id'")
        status = str(row.get("status", "")).strip().lower()
        if not status:
            raise ValueError(f"{path}: module '{module_id}' missing 'status'")

        failing_guardrails = frozenset(
            str(v).strip() for v in row.get("failing_guardrails", []) if str(v).strip()
        )
        statuses.append(
            ModuleStatus(
                module_id=module_id,
                status=status,
                failing_guardrails=failing_guardrails,
                remediation_issue=str(row.get("remediation_issue", "")).strip(),
                remediation_notes=str(row.get("remediation_notes", "")).strip(),
            )
        )
    return statuses


def validate_module_statuses(
    path: Path,
    module_statuses: list[ModuleStatus],
    required_ids: set[str],
    policy: DeprecationGatePolicy,
) -> list[str]:
    errors: list[str] = []
    seen_ids: set[str] = set()

    for module in module_statuses:
        if module.module_id in seen_ids:
            errors.append(f"{path}: duplicate module id '{module.module_id}'")
            continue
        seen_ids.add(module.module_id)

        if module.status not in policy.allowed_statuses:
            errors.append(
                f"{path}: module '{module.module_id}' has invalid status '{module.status}' "
                f"(allowed: {sorted(policy.allowed_statuses)})"
            )
            continue

        unknown_failing = sorted(g for g in module.failing_guardrails if g not in required_ids)
        if unknown_failing:
            errors.append(
                f"{path}: module '{module.module_id}' lists unknown failing guardrails {unknown_failing}"
            )

        if module.status == "active" and module.failing_guardrails:
            errors.append(
                f"{path}: module '{module.module_id}' is active but has failing guardrails "
                f"{sorted(module.failing_guardrails)}; set status to 'deprecated'"
            )

        if module.status == "deprecated" and not module.failing_guardrails:
            errors.append(
                f"{path}: module '{module.module_id}' is deprecated but has no failing guardrails "
                f"to justify deprecation"
            )

        if module.status == "remediation" and module.failing_guardrails:
            errors.append(
                f"{path}: module '{module.module_id}' is in remediation but still has failing "
                f"guardrails {sorted(module.failing_guardrails)}; keep it deprecated until failures clear"
            )

        if (
            policy.require_remediation_issue_for_non_active
            and module.status != "active"
            and not module.remediation_issue
        ):
            errors.append(
                f"{path}: module '{module.module_id}' status '{module.status}' "
                f"requires remediation_issue"
            )

        if module.status == "remediation" and not module.remediation_notes:
            errors.append(
                f"{path}: module '{module.module_id}' status 'remediation' requires remediation_notes"
            )

        if module.status in policy.release_block_statuses:
            errors.append(
                f"{path}: module '{module.module_id}' status '{module.status}' blocks release "
                f"until remediation is complete"
            )

    return errors


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
    module_status_path: Path | None = None,
) -> list[str]:
    payload = _load_payload(guardrails_path)
    guardrails = _parse_guardrails(payload, guardrails_path)
    policy = _parse_deprecation_gate_policy(payload, guardrails_path)

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

    if policy.enabled and module_status_path is not None:
        if not module_status_path.exists():
            errors.append(f"{module_status_path}: module status file is missing")
        else:
            module_statuses = load_module_statuses(module_status_path)
            if not module_statuses:
                errors.append(f"{module_status_path}: expected at least one [[module]] entry")
            else:
                errors.extend(
                    validate_module_statuses(
                        module_status_path,
                        module_statuses,
                        required_ids,
                        policy,
                    )
                )

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
    parser.add_argument(
        "--module-status-file",
        type=Path,
        default=Path("docs/governance/module-guardrail-status.toml"),
        help="Path to DeFi module status file used by deprecation gate (relative to --repo-root).",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    repo_root = args.repo_root.resolve()
    guardrails_path = (repo_root / args.guardrails_file).resolve()
    release_gate_path = (repo_root / args.release_gate_file).resolve()
    proposals_dir = (repo_root / args.proposals_dir).resolve()
    module_status_path = (repo_root / args.module_status_file).resolve()

    try:
        errors = collect_errors(
            guardrails_path=guardrails_path,
            release_gate_path=release_gate_path,
            proposals_dir=proposals_dir,
            module_status_path=module_status_path,
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
    print(f"Checked: {module_status_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
