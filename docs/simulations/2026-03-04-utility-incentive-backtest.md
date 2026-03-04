# Utility Incentive Backtest (defi-protocols#7)

Date: 2026-03-04

Command:

```bash
python3 scripts/simulate_utility_incentives.py
```

## Scenario Output

| Label | Base Reward | Aggregate Score (bps) | Multiplier (bps) | Weighted Reward | Class |
| --- | ---: | ---: | ---: | ---: | --- |
| high-utility-forecast-a | 1000 | 8825 | 13825 | 1382 | utility_linked |
| high-utility-report-b | 900 | 8151 | 13151 | 1183 | utility_linked |
| moderate-utility-c | 1000 | 6700 | 11700 | 1170 | utility_linked |
| financial-drift-c | 1100 | 2427 | 7942 | 873 | financial_activity |
| financial-drift-d | 950 | 1995 | 7596 | 721 | financial_activity |

## Totals

- baseline_total: `4950`
- weighted_total: `5329`
- utility_linked_total: `3735`
- financial_activity_total: `1594`
- utility_reward_ratio_bps: `7008`

Interpretation:
- Weighted incentives preferentially route rewards toward higher verified utility signals.
- In this deterministic scenario, ~70.08% of weighted rewards are utility-linked.
