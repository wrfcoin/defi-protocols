#!/usr/bin/env python3
from __future__ import annotations

from dataclasses import dataclass

BPS = 10_000


@dataclass(frozen=True)
class Window:
    label: str
    participant_class: str
    raw_participant_identifier_exposures: int
    raw_amount_exposures: int
    high_granularity_timestamp_exposures: int
    out_of_scope_disclosures: int
    total_reward_flow_events: int


def metadata_leakage_risk_score_bps(window: Window) -> int:
    if window.total_reward_flow_events <= 0:
        return 0

    def normalize(count: int) -> int:
        return min(BPS, (count * BPS) // window.total_reward_flow_events)

    id_risk = normalize(window.raw_participant_identifier_exposures)
    amount_risk = normalize(window.raw_amount_exposures)
    ts_risk = normalize(window.high_granularity_timestamp_exposures)
    scope_risk = normalize(window.out_of_scope_disclosures)

    weighted = (
        id_risk * 3500
        + amount_risk * 2500
        + ts_risk * 2000
        + scope_risk * 2000
    )
    return min(BPS, weighted // BPS)


def main() -> int:
    windows = [
        Window("cohort-safe-h", "human", 1, 1, 3, 0, 4000),
        Window("cohort-safe-a", "ai_agent", 1, 1, 3, 0, 4000),
        Window("cohort-safe-r", "robot", 1, 1, 3, 0, 4000),
        Window("moderate-leakage-h", "human", 25, 28, 40, 3, 4500),
        Window("moderate-leakage-a", "ai_agent", 25, 28, 40, 3, 4500),
        Window("moderate-leakage-r", "robot", 25, 28, 40, 3, 4500),
        Window("high-leakage-mixed", "mixed", 140, 130, 180, 22, 5000),
    ]

    print(
        "label,participant_class,id_exposures,amount_exposures,timestamp_exposures,"
        "scope_exposures,total_events,metadata_leakage_risk_score_bps"
    )

    total_id = 0
    total_amount = 0
    total_ts = 0
    total_scope = 0
    total_events = 0

    scores_by_label = {}
    for window in windows:
        score = metadata_leakage_risk_score_bps(window)
        scores_by_label[window.label] = score
        total_id += window.raw_participant_identifier_exposures
        total_amount += window.raw_amount_exposures
        total_ts += window.high_granularity_timestamp_exposures
        total_scope += window.out_of_scope_disclosures
        total_events += window.total_reward_flow_events
        print(
            f"{window.label},{window.participant_class},"
            f"{window.raw_participant_identifier_exposures},"
            f"{window.raw_amount_exposures},"
            f"{window.high_granularity_timestamp_exposures},"
            f"{window.out_of_scope_disclosures},"
            f"{window.total_reward_flow_events},{score}"
        )

    aggregate = Window(
        label="aggregate",
        participant_class="all",
        raw_participant_identifier_exposures=total_id,
        raw_amount_exposures=total_amount,
        high_granularity_timestamp_exposures=total_ts,
        out_of_scope_disclosures=total_scope,
        total_reward_flow_events=total_events,
    )
    aggregate_score = metadata_leakage_risk_score_bps(aggregate)

    equal_severity_safe = (
        scores_by_label["cohort-safe-h"]
        == scores_by_label["cohort-safe-a"]
        == scores_by_label["cohort-safe-r"]
    )
    equal_severity_moderate = (
        scores_by_label["moderate-leakage-h"]
        == scores_by_label["moderate-leakage-a"]
        == scores_by_label["moderate-leakage-r"]
    )

    print()
    print(f"aggregate_id_exposures={total_id}")
    print(f"aggregate_amount_exposures={total_amount}")
    print(f"aggregate_timestamp_exposures={total_ts}")
    print(f"aggregate_scope_exposures={total_scope}")
    print(f"aggregate_total_events={total_events}")
    print(f"metadata_leakage_risk_score_bps={aggregate_score}")
    print(f"equal_severity_safe_alignment={equal_severity_safe}")
    print(f"equal_severity_moderate_alignment={equal_severity_moderate}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
