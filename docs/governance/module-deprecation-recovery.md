# DeFi Module Deprecation And Recovery Path

Date: 2026-03-04  
Owner: `defi-maintainers`

## Why This Exists

Issue `#10` requires automatic deprecation whenever a DeFi module fails one or more required
mission/privacy guardrails. The deprecation gate is enforced by:

- `docs/governance/mission-guardrails.toml` (`[deprecation_gate]` section)
- `docs/governance/module-guardrail-status.toml` (`[[module]]` status inputs)
- `scripts/check_mission_guardrails.py` (CI/policy validator)

## Status Model

Allowed statuses are:

- `active`: module has no required guardrail failures.
- `deprecated`: module currently fails one or more required guardrails and is release-blocking.
- `remediation`: module has cleared failing guardrails and is in monitored recovery before returning
  to `active`.

## Automatic Deprecation Rules

1. Any module with non-empty `failing_guardrails` must not be `active`.
2. A `deprecated` module must list at least one failing required guardrail.
3. `deprecated` status blocks release while active.
4. Any non-`active` module must include a `remediation_issue` reference.
5. `remediation` status requires `remediation_notes` and zero active failing guardrails.

## Recovery Workflow

1. Open/update remediation issue in `wrfcoin/defi-protocols` and link it in
   `module-guardrail-status.toml`.
2. Implement fixes that clear each listed guardrail failure.
3. Update `failing_guardrails = []` and set status to `remediation` with evidence notes.
4. Run:
   - `python3 scripts/check_mission_guardrails.py --repo-root .`
   - `python3 -m unittest scripts/test_check_mission_guardrails.py`
5. After monitored validation window and reviewer approval, set status to `active`.

## Equal-Liability Reminder

Deprecation and remediation criteria are actor-class neutral and apply equally to humans,
AI agents, and robots under equivalent misconduct or policy drift.
