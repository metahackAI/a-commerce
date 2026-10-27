"""Periods by name, resolved against the store clock, so a model never has to compute a date."""

from __future__ import annotations

from datetime import datetime, timedelta
from typing import Literal

Period = Literal["this_weekend", "next_weekend", "today", "tomorrow", "next_7_days"]
PERIOD_NAMES = ("this_weekend", "next_weekend", "today", "tomorrow", "next_7_days")
PERIOD_HELP = "Or pass period instead of dates: this_weekend, next_weekend, today, tomorrow, next_7_days."


def resolve(period: str, now: datetime) -> tuple[str, str]:
    day = now.replace(hour=0, minute=0, second=0, microsecond=0)
    if period in ("this_weekend", "next_weekend"):
        sat = day + timedelta(days=(5 - day.weekday()) % 7) if day.weekday() != 6 else day - timedelta(days=1)
        if period == "next_weekend":
            sat += timedelta(days=7)
        start, end = sat, sat + timedelta(days=1, hours=23, minutes=59)
    elif period == "today":
        start, end = day, day + timedelta(hours=23, minutes=59)
    elif period == "tomorrow":
        start, end = day + timedelta(days=1), day + timedelta(days=1, hours=23, minutes=59)
    elif period == "next_7_days":
        start, end = day, day + timedelta(days=6, hours=23, minutes=59)
    else:
        raise ValueError(period)
    return start.isoformat(timespec="minutes"), end.isoformat(timespec="minutes")
