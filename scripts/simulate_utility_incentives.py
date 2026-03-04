#!/usr/bin/env python3
from __future__ import annotations

from dataclasses import dataclass

BPS = 10_000


@dataclass(frozen=True)
class Config:
    min_multiplier_bps: int = 6_000
    neutral_multiplier_bps: int = 10_000
    max_multiplier_bps: int = 15_000
    utility_score_weight_bps: int = 7_000
    verified_ratio_weight_bps: int = 3_000
    utility_linked_threshold_bps: int = 6_000


@dataclass(frozen=True)
class Point:
    label: str
    base_reward: int
    forecast_usefulness_bps: int
    report_usefulness_bps: int
    confidence_bps: int
    verified_events: int
    unverified_events: int


def composite_score(point: Point) -> int:
    avg = (point.forecast_usefulness_bps + point.report_usefulness_bps) // 2
    usefulness = (avg * min(point.confidence_bps, BPS)) // BPS
    total_events = point.verified_events + point.unverified_events
    verified_ratio = BPS // 2 if total_events == 0 else min(
        BPS, (point.verified_events * BPS) // total_events
    )
    return usefulness, verified_ratio


def aggregate_score(config: Config, point: Point) -> int:
    usefulness, verified_ratio = composite_score(point)
    weighted = (
        usefulness * config.utility_score_weight_bps
        + verified_ratio * config.verified_ratio_weight_bps
    )
    return weighted // BPS


def multiplier_bps(config: Config, score_bps: int) -> int:
    neutral_score = BPS // 2
    if score_bps >= neutral_score:
        up_span = config.max_multiplier_bps - config.neutral_multiplier_bps
        return min(
            config.max_multiplier_bps,
            config.neutral_multiplier_bps
            + ((up_span * (score_bps - neutral_score)) // neutral_score),
        )
    down_span = config.neutral_multiplier_bps - config.min_multiplier_bps
    return max(
        config.min_multiplier_bps,
        config.neutral_multiplier_bps
        - ((down_span * (neutral_score - score_bps)) // neutral_score),
    )


def main() -> int:
    config = Config()
    points = [
        Point("high-utility-forecast-a", 1000, 9000, 8500, 10000, 9, 1),
        Point("high-utility-report-b", 900, 8200, 9100, 9500, 8, 2),
        Point("moderate-utility-c", 1000, 7000, 7000, 10000, 6, 4),
        Point("financial-drift-c", 1100, 2800, 3000, 9000, 2, 8),
        Point("financial-drift-d", 950, 2500, 2600, 9500, 1, 9),
    ]

    baseline_total = 0
    weighted_total = 0
    utility_linked_total = 0
    financial_total = 0

    print("label,base_reward,aggregate_score_bps,multiplier_bps,weighted_reward,class")
    for point in points:
        score = aggregate_score(config, point)
        multiplier = multiplier_bps(config, score)
        weighted = (point.base_reward * multiplier) // BPS
        reward_class = (
            "utility_linked"
            if score >= config.utility_linked_threshold_bps
            else "financial_activity"
        )

        baseline_total += point.base_reward
        weighted_total += weighted
        if reward_class == "utility_linked":
            utility_linked_total += weighted
        else:
            financial_total += weighted

        print(
            f"{point.label},{point.base_reward},{score},{multiplier},{weighted},{reward_class}"
        )

    utility_ratio_bps = 0 if weighted_total == 0 else (utility_linked_total * BPS) // weighted_total
    print()
    print(f"baseline_total={baseline_total}")
    print(f"weighted_total={weighted_total}")
    print(f"utility_linked_total={utility_linked_total}")
    print(f"financial_activity_total={financial_total}")
    print(f"utility_reward_ratio_bps={utility_ratio_bps}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
