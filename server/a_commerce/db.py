"""SQLite store, demo clock, and policy access."""

from __future__ import annotations

import json
import sqlite3
import threading
from datetime import datetime
from pathlib import Path
from typing import Any

import yaml

POLICY_PATH = Path(__file__).with_name("policy.yaml")

SCHEMA = """
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE categories (id TEXT PRIMARY KEY, name TEXT NOT NULL);

CREATE TABLE products (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  category_id TEXT NOT NULL REFERENCES categories(id),
  color TEXT, fabric TEXT,
  price REAL NOT NULL, cost REAL NOT NULL,
  weight_g INTEGER, packed TEXT, warmth TEXT, light_rain INTEGER,
  colors INTEGER NOT NULL DEFAULT 1,
  look TEXT,
  status TEXT NOT NULL DEFAULT 'live'
);

CREATE TABLE variants (
  id TEXT PRIMARY KEY,
  product_id TEXT NOT NULL REFERENCES products(id),
  size TEXT NOT NULL,
  stock INTEGER NOT NULL
);

CREATE TABLE tiers (id TEXT PRIMARY KEY, name TEXT NOT NULL);

CREATE TABLE members (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  tier_id TEXT NOT NULL REFERENCES tiers(id),
  profile TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE preferences (
  id TEXT PRIMARY KEY,
  member_id TEXT NOT NULL,
  key TEXT NOT NULL, value TEXT NOT NULL,
  source TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL
);

CREATE TABLE promotions (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,              -- sale, shipping, coupon, tier_coupon, bundle
  audience TEXT NOT NULL,          -- everyone, members, vip, gold, ...
  starts_at TEXT, ends_at TEXT,    -- NULL means always
  status TEXT NOT NULL,            -- live, undone
  summary TEXT
);

CREATE TABLE promotion_items (
  promotion_id TEXT NOT NULL REFERENCES promotions(id),
  product_id TEXT NOT NULL REFERENCES products(id),
  percent_off REAL NOT NULL,
  sale_price REAL NOT NULL,
  PRIMARY KEY (promotion_id, product_id)
);

CREATE TABLE coupons (
  id TEXT PRIMARY KEY,
  code TEXT NOT NULL,
  promotion_id TEXT REFERENCES promotions(id),
  audience TEXT NOT NULL,
  auto_apply INTEGER NOT NULL,
  stacking TEXT NOT NULL,
  issued_count INTEGER NOT NULL,
  status TEXT NOT NULL
);

CREATE TABLE display_slots (id TEXT PRIMARY KEY, page TEXT NOT NULL, slot TEXT NOT NULL);

CREATE TABLE displays (
  id TEXT PRIMARY KEY,
  slot_id TEXT NOT NULL REFERENCES display_slots(id),
  audience TEXT NOT NULL,
  starts_at TEXT, ends_at TEXT,
  eyebrow TEXT, headline TEXT NOT NULL, cta TEXT, link TEXT,
  promotion_id TEXT,
  status TEXT NOT NULL
);

CREATE TABLE previews (
  id TEXT PRIMARY KEY,
  tool TEXT NOT NULL,
  args TEXT NOT NULL,
  result TEXT NOT NULL,
  status TEXT NOT NULL,            -- open, committed, discarded
  committed_id TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE change_sets (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL,
  version INTEGER NOT NULL,
  requests TEXT NOT NULL,
  preview_ids TEXT NOT NULL,
  undo_until TEXT,
  verification TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE carts (id TEXT PRIMARY KEY, member_id TEXT NOT NULL);
CREATE TABLE cart_lines (
  cart_id TEXT NOT NULL, product_id TEXT NOT NULL, size TEXT NOT NULL, qty INTEGER NOT NULL,
  PRIMARY KEY (cart_id, product_id, size)
);

CREATE TABLE orders (
  id TEXT PRIMARY KEY,
  member_id TEXT NOT NULL,
  status TEXT NOT NULL,
  placed_at TEXT, delivered_at TEXT,
  lines TEXT NOT NULL,
  subtotal REAL NOT NULL, benefit REAL NOT NULL, shipping REAL NOT NULL, total REAL NOT NULL,
  coupon_code TEXT,
  payment TEXT, ship_to TEXT
);

CREATE TABLE return_requests (
  id TEXT PRIMARY KEY,
  order_id TEXT NOT NULL,
  member_id TEXT NOT NULL,
  item TEXT NOT NULL,
  amount REAL NOT NULL,
  reason TEXT NOT NULL,            -- defect, damaged, too_small, too_large, color, changed_mind
  reason_text TEXT NOT NULL,
  photos INTEGER NOT NULL,
  delivered_at TEXT NOT NULL,
  requested_at TEXT NOT NULL,
  option TEXT NOT NULL,            -- refund, replacement
  status TEXT NOT NULL             -- waiting, refunded, photos_requested, declined
);

CREATE TABLE refunds (
  id TEXT PRIMARY KEY, return_request_id TEXT NOT NULL, amount REAL NOT NULL, to_payment TEXT NOT NULL,
  status TEXT NOT NULL, created_at TEXT NOT NULL
);

CREATE TABLE messages (
  id TEXT PRIMARY KEY, member_id TEXT NOT NULL, return_request_id TEXT, kind TEXT NOT NULL, text TEXT NOT NULL,
  status TEXT NOT NULL, created_at TEXT NOT NULL
);

CREATE TABLE action_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  detail TEXT NOT NULL
);
"""


class Store:
    """One SQLite database plus the demo clock. Thread-safe enough for a demo server."""

    def __init__(self, path: str = ":memory:"):
        self.conn = sqlite3.connect(path, check_same_thread=False)
        self.conn.row_factory = sqlite3.Row
        self.lock = threading.RLock()
        self.policy = load_policy()
        self._ids: dict[str, int] = {}

    # -- schema and clock -------------------------------------------------
    def create_schema(self) -> None:
        self.conn.executescript(SCHEMA)

    def now(self) -> datetime:
        row = self.one("SELECT value FROM meta WHERE key = 'clock'")
        return datetime.fromisoformat(row["value"])

    def set_clock(self, at: datetime | str) -> None:
        value = at if isinstance(at, str) else at.isoformat(timespec="minutes")
        datetime.fromisoformat(value)  # validate
        self.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('clock', ?)", (value,))

    def new_id(self, prefix: str) -> str:
        row = self.one("SELECT value FROM meta WHERE key = ?", (f"seq:{prefix}",))
        n = int(row["value"]) + 1 if row else 1
        self.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", (f"seq:{prefix}", str(n)))
        return f"{prefix}_{n}"

    # -- queries ----------------------------------------------------------
    def execute(self, sql: str, params: tuple | list = ()) -> sqlite3.Cursor:
        with self.lock:
            cur = self.conn.execute(sql, params)
            self.conn.commit()
            return cur

    def executemany(self, sql: str, rows: list[tuple]) -> None:
        with self.lock:
            self.conn.executemany(sql, rows)
            self.conn.commit()

    def all(self, sql: str, params: tuple | list = ()) -> list[sqlite3.Row]:
        with self.lock:
            return self.conn.execute(sql, params).fetchall()

    def one(self, sql: str, params: tuple | list = ()) -> sqlite3.Row | None:
        with self.lock:
            return self.conn.execute(sql, params).fetchone()

    def log(self, actor: str, action: str, detail: dict[str, Any]) -> None:
        self.execute(
            "INSERT INTO action_log (at, actor, action, detail) VALUES (?, ?, ?, ?)",
            (self.now().isoformat(timespec="minutes"), actor, action, json.dumps(detail)),
        )

    def policy_value(self, name: str) -> Any:
        return self.policy[name]["value"]


def load_policy() -> dict[str, Any]:
    return yaml.safe_load(POLICY_PATH.read_text())


def money(x: float) -> float:
    """Round to cents, half up, so 79 * 0.85 is 67.15 and not 67.14."""
    from decimal import ROUND_HALF_UP, Decimal

    return float(Decimal(str(x)).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP))
