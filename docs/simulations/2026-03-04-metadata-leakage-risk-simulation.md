# Metadata Leakage Risk Simulation (defi-protocols#8)

Date: 2026-03-04

Command:

```bash
python3 -B scripts/score_metadata_leakage_risk.py
```

## Scenario Output

| Label | Participant Class | ID Exposures | Amount Exposures | Timestamp Exposures | Scope Exposures | Total Events | Risk Score (bps) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cohort-safe-h | human | 1 | 1 | 3 | 0 | 4000 | 2 |
| cohort-safe-a | ai_agent | 1 | 1 | 3 | 0 | 4000 | 2 |
| cohort-safe-r | robot | 1 | 1 | 3 | 0 | 4000 | 2 |
| moderate-leakage-h | human | 25 | 28 | 40 | 3 | 4500 | 53 |
| moderate-leakage-a | ai_agent | 25 | 28 | 40 | 3 | 4500 | 53 |
| moderate-leakage-r | robot | 25 | 28 | 40 | 3 | 4500 | 53 |
| high-leakage-mixed | mixed | 140 | 130 | 180 | 22 | 5000 | 243 |

## Aggregate KPI

- aggregate_id_exposures: `218`
- aggregate_amount_exposures: `217`
- aggregate_timestamp_exposures: `309`
- aggregate_scope_exposures: `31`
- aggregate_total_events: `30500`
- metadata_leakage_risk_score_bps: `64`

## Policy Alignment Check

- equal_severity_safe_alignment: `True`
- equal_severity_moderate_alignment: `True`

Interpretation:
- Identical-severity windows for humans, AI agents, and robots produce identical risk scores.
- This supports equal-liability privacy treatment across participant classes.
