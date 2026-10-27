"""Deterministic seed data. Every number here comes from docs/plan.md §4; tests check them."""

from __future__ import annotations

import json

from .db import Store, money

SEED_CLOCK = "2026-11-06T11:00"

CATEGORIES = [
    ("summer_outerwear", "Summer outerwear"), ("coats", "Coats"), ("knitwear", "Knitwear"),
    ("shirts", "Shirts"), ("denim", "Denim"), ("footwear", "Footwear"), ("active", "Active"),
    ("trousers", "Trousers"), ("tees", "Tees"), ("sweatshirts", "Sweatshirts"), ("shorts", "Shorts"),
    ("dresses", "Dresses"), ("skirts", "Skirts"), ("suits", "Suits"), ("loungewear", "Loungewear"),
    ("underwear", "Underwear"), ("socks", "Socks"), ("hats", "Hats"), ("scarves", "Scarves"),
    ("bags", "Bags"), ("belts", "Belts"), ("swim", "Swim"), ("rainwear", "Rainwear"), ("gifts", "Gift cards"),
]

SIZES = ("S", "M", "L", "XL")

# id, name, color, fabric, price, cost, stock by size (S, M, L, XL), weight_g, packed, warmth, light_rain, colors, look
SUMMER_OUTERWEAR = [
    ("prd_field_jacket", "Linen blend field jacket", "Sand", "linen, cotton", 128.00, 51.20, (18, 26, 24, 18), 340, "Folds flat", "Light", 0, 3, "With chinos, sleeves rolled"),
    ("prd_windbreaker", "Packable windbreaker", "Slate", "recycled nylon", 89.00, 35.60, (44, 62, 60, 48), 190, "Into its own pocket, 14 × 18 cm", "Wind layer only", 1, 4, "Over a tee for windy days"),
    ("prd_overshirt", "Seersucker overshirt", "Clay", "cotton", 79.00, 56.00, (12, 16, 15, 10), 360, "Folds flat", "Light", 0, 2, "Open, as a light layer"),
    ("prd_utility_jacket", "Cotton utility jacket", "Olive", "cotton", 118.00, 47.20, (6, 10, 9, 6), 480, "Folds flat", "Medium", 0, 2, "Over a knit on cool mornings"),
    ("prd_camp_overshirt", "Linen camp overshirt", "Stone", "linen", 88.00, 35.20, (26, 36, 34, 24), 300, "Folds flat", "Light", 0, 3, "Unbuttoned over a tank"),
    ("prd_chore_coat", "Cotton chore coat", "Indigo", "cotton", 142.00, 56.80, (4, 0, 0, 3), 620, "Folds flat", "Medium", 0, 2, "Boxy fit, goes with denim"),
    ("prd_rain_parka", "Light rain parka", "Black", "coated nylon", 169.00, 67.60, (2, 0, 0, 2), 420, "Rolls into its hood", "Light", 1, 2, "Over everything on wet days"),
]

GEN_FABRICS = ["Cotton", "Linen", "Nylon", "Ripstop", "Chambray", "Hemp", "Poplin"]
GEN_TYPES = ["shirt jacket", "bomber", "coach jacket", "anorak", "blouson"]
GEN_PRICES = [98.00, 108.00, 118.00, 128.00, 138.00, 148.00, 88.00, 78.00]
GEN_COLORS = ["Sand", "Navy", "Olive", "Ecru", "Rust", "Slate", "Black"]

# id, name, category, color, fabric, price, cost, stock by size
OTHER_PRODUCTS = [
    ("prd_overcoat", "Wool overcoat", "coats", "Charcoal", "70% wool", 289.00, 115.60, {"S": 8, "M": 12, "L": 10, "XL": 6}),
    ("prd_chelsea_boots", "Leather chelsea boots", "footwear", "Tan", "leather", 312.00, 124.80, {"8": 5, "9": 6, "10": 7, "11": 4}),
    ("prd_cashmere", "Cashmere sweater", "knitwear", "Oatmeal", "cashmere", 248.00, 99.20, {"S": 9, "M": 14, "L": 11, "XL": 5}),
    ("prd_denim_jacket", "Denim jacket", "denim", "Mid blue", "cotton denim", 118.00, 47.20, {"S": 9, "M": 0, "L": 0, "XL": 7}),
    ("prd_linen_shirt", "Linen shirt", "shirts", "White", "linen", 69.00, 27.60, {"S": 10, "M": 16, "L": 12, "XL": 0}),
    ("prd_fleece", "Fleece half-zip", "active", "Moss", "recycled fleece", 84.00, 33.60, {"S": 20, "M": 30, "L": 28, "XL": 18}),
    ("prd_merino_tee", "Merino trail tee", "active", "Charcoal", "merino", 58.00, 23.20, {"S": 25, "M": 40, "L": 35, "XL": 20}),
]

TIER_COUNTS = {"vip": 1284, "gold": 2410, "member": 9870}

# id, name, kind, audience, starts_at, ends_at, summary, items [(product_id, percent)]
PROMOTIONS = [
    ("promo_autumn_knit", "Autumn knitwear 15%", "sale", "everyone", "2026-11-01T00:00", "2026-11-15T23:59", "15% off knitwear", [("prd_cashmere", 15)]),
    ("promo_free_ship", "Free shipping over $80", "shipping", "everyone", None, None, "Free shipping on subtotals of $80 or more", []),
    ("promo_birthday", "Birthday coupon", "coupon", "members", None, None, "A coupon in each member's birthday month", []),
    ("promo_gold_10", "Gold tier 10%", "tier_coupon", "gold", None, None, "10% off for Gold members", []),
    ("promo_denim_bundle", "Denim bundle", "bundle", "everyone", "2026-11-13T00:00", "2026-11-20T23:59", "Jacket and jeans together", []),
    ("promo_linen_clear", "Linen clearance", "sale", "everyone", "2026-10-01T00:00", "2026-10-20T23:59", "Ended", []),
]


def seed(store: Store) -> None:
    store.create_schema()
    store.set_clock(SEED_CLOCK)
    # Sequences start after the ids the seed writes by hand.
    for prefix, start in (("cs", 1041), ("order", 20431), ("pref", 3)):
        store.execute("INSERT INTO meta (key, value) VALUES (?, ?)", (f"seq:{prefix}", str(start)))

    store.executemany("INSERT INTO categories VALUES (?, ?)", CATEGORIES)
    _products(store)
    _members(store)
    _promotions(store)
    _display(store)
    _orders(store)
    _returns(store)


def _products(store: Store) -> None:
    rows, variants = [], []
    for (pid, name, color, fabric, price, cost, stock, weight, packed, warmth, rain, colors, look) in SUMMER_OUTERWEAR:
        rows.append((pid, name, "summer_outerwear", color, fabric, price, cost, weight, packed, warmth, rain, colors, look))
        variants += [(f"{pid}_{s}", pid, s, n) for s, n in zip(SIZES, stock)]

    n = 0
    for fabric in GEN_FABRICS:
        for kind in GEN_TYPES:
            n += 1
            pid = f"prd_so_{n:02d}"
            price = GEN_PRICES[n % len(GEN_PRICES)]
            stock = 12 + (n * 37) % 60
            rows.append((pid, f"{fabric} {kind}", "summer_outerwear", GEN_COLORS[n % len(GEN_COLORS)], fabric.lower(),
                         price, money(price * 0.4), 250 + (n * 13) % 250, "Folds flat", "Light", 0, 2, None))
            variants += [(f"{pid}_{s}", pid, s, stock) for s in SIZES]

    for (pid, name, cat, color, fabric, price, cost, stock) in OTHER_PRODUCTS:
        rows.append((pid, name, cat, color, fabric, price, cost, None, None, None, 0, 1, None))
        variants += [(f"{pid}_{s}", pid, s, q) for s, q in stock.items()]

    store.executemany(
        "INSERT INTO products (id, name, category_id, color, fabric, price, cost, weight_g, packed, warmth, light_rain, colors, look)"
        " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", rows)
    store.executemany("INSERT INTO variants VALUES (?, ?, ?, ?)", variants)


def _members(store: Store) -> None:
    store.executemany("INSERT INTO tiers VALUES (?, ?)", [("vip", "VIP"), ("gold", "Gold"), ("member", "Member")])
    jamie = {
        "home": "12 Harbor St, Apt 3", "card_last4": "4821", "orders": 6,
    }
    alex = {"home": "77 Ferry Ln", "card_last4": "0937", "orders": 3}
    store.executemany("INSERT INTO members (id, name, tier_id, profile) VALUES (?, ?, ?, ?)", [
        ("mem_jamie", "Jamie", "vip", json.dumps(jamie)),
        ("mem_alex", "Alex", "member", json.dumps(alex)),
    ])
    rows = []
    for tier, count in TIER_COUNTS.items():
        named = 1  # Jamie is VIP, Alex is a member
        for i in range(count - named if tier != "gold" else count):
            rows.append((f"mem_{tier}_{i:05d}", f"{tier.title()} member {i}", tier, "{}"))
    store.executemany("INSERT INTO members (id, name, tier_id, profile) VALUES (?, ?, ?, ?)", rows)

    store.executemany(
        "INSERT INTO preferences (id, member_id, key, value, source, created_at) VALUES (?, ?, ?, ?, ?, ?)", [
            ("pref_1", "mem_jamie", "size", "M", "profile", "2026-03-02T10:00"),
            ("pref_2", "mem_jamie", "fabric", "Natural fabrics", "profile", "2026-04-11T10:00"),
            ("pref_3", "mem_jamie", "view", "Compares specs", "profile", "2026-05-20T10:00"),
        ])


def _promotions(store: Store) -> None:
    for (pid, name, kind, audience, start, end, summary, items) in PROMOTIONS:
        store.execute("INSERT INTO promotions VALUES (?, ?, ?, ?, ?, ?, 'live', ?)",
                      (pid, name, kind, audience, start, end, summary))
        for product_id, pct in items:
            price = store.one("SELECT price FROM products WHERE id = ?", (product_id,))["price"]
            store.execute("INSERT INTO promotion_items VALUES (?, ?, ?, ?)",
                          (pid, product_id, pct, money(price * (100 - pct) / 100)))
    store.execute("INSERT INTO coupons VALUES ('coupon_gold10', 'GOLD10', 'promo_gold_10', 'gold', 1, 'tier', ?, 'live')",
                  (TIER_COUNTS["gold"],))


def _display(store: Store) -> None:
    store.execute("INSERT INTO display_slots VALUES ('slot_main_hero', 'main', 'hero')")
    store.execute(
        "INSERT INTO displays VALUES ('disp_fall_layers', 'slot_main_hero', 'everyone', '2026-11-06T08:10', NULL,"
        " 'NEW SEASON', 'Fall layers', 'Shop layers', '/c/coats', NULL, 'live')")


def _orders(store: Store) -> None:
    def line(pid: str, size: str, price: float, qty: int = 1) -> dict:
        return {"product_id": pid, "size": size, "qty": qty, "price": price}

    jamie = [
        ("#20011", "2026-03-02T10:00", [line("prd_linen_shirt", "M", 69.00)]),
        ("#20057", "2026-04-11T10:00", [line("prd_merino_tee", "M", 58.00)]),
        ("#20103", "2026-05-20T10:00", [line("prd_fleece", "L", 84.00)]),
        ("#20166", "2026-07-08T10:00", [line("prd_so_03", "M", 128.00)]),
        ("#20221", "2026-09-14T10:00", [line("prd_camp_overshirt", "M", 88.00)]),
        ("#20312", "2026-10-19T10:00", [line("prd_cashmere", "M", 248.00)]),
    ]
    rows = []
    for oid, at, lines in jamie:
        total = sum(item["price"] * item["qty"] for item in lines)
        rows.append((oid, "mem_jamie", "delivered", at, at, json.dumps(lines), total, 0.0, 0.0, total, None,
                     "card_4821", "home"))
    rows.append(("#20402", "mem_alex", "delivered", "2026-10-28T14:00", "2026-10-31T13:00",
                 json.dumps([line("prd_chelsea_boots", "9", 312.00)]), 312.00, 0.0, 0.0, 312.00, None, "card_0937", "home"))
    store.executemany("INSERT INTO orders VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", rows)


def fresh_store(path: str = ":memory:") -> Store:
    store = Store(path)
    seed(store)
    return store


# order, item, amount, reason, reason text, photos, delivered, coupon used  (docs/plan.md §4.9; clock Fri Nov 6)
RETURNS = [
    ("#20431", "Wool overcoat", 289.00, "color", "Color differs from photo", 1, "2026-11-02T13:00", None),
    ("#20418", "Linen blend field jacket ×2", 256.00, "too_small", "Too small", 0, "2026-10-30T13:00", "BIRTHDAY10"),
    ("#20377", "Field jacket + windbreaker", 217.00, "changed_mind", "Changed mind", 0, "2026-10-28T13:00", "BIRTHDAY10"),
    ("#20366", "Rain parka ×2", 338.00, "damaged", "Arrived damaged", 0, "2026-11-04T13:00", None),
    ("#20290", "Cashmere sweater", 248.00, "changed_mind", "Changed mind", 0, "2026-09-28T13:00", None),
    ("#20380", "Wool overcoat", 315.00, "defect", "Zipper broke", 1, "2026-11-01T13:00", None),
    ("#20385", "Cotton chore coat ×2", 302.00, "too_large", "Too large", 0, "2026-10-29T13:00", None),
    ("#20391", "Utility jacket + linen shirt", 276.00, "changed_mind", "Changed mind", 0, "2026-10-25T13:00", "GOLD10"),
    ("#20396", "Field jacket + tee", 268.00, "color", "Color differs from photo", 1, "2026-11-03T13:00", None),
    ("#20399", "Linen shirts ×4", 228.00, "too_small", "Too small", 0, "2026-10-27T13:00", None),
    ("#20405", "Merino trail tees ×4", 244.00, "damaged", "Arrived damaged", 0, "2026-11-05T13:00", None),
]
ALEX_RETURN = ("#20402", "mem_alex", "Leather chelsea boots", 312.00, "defect", "Defect: sole coming apart", 2,
               "2026-10-31T13:00")


def _returns(store: Store) -> None:
    """Eleven waiting returns over $200. Alex's boots (#20402) become the twelfth when Alex sends the request."""
    for i, (oid, item, amount, reason, text, photos, delivered, coupon) in enumerate(RETURNS, start=1):
        mid = f"mem_ret_{oid[1:]}"
        store.execute("INSERT INTO members (id, name, tier_id, profile) VALUES (?, ?, 'member', ?)",
                      (mid, f"Customer {oid}", json.dumps({"card_last4": f"{1000 + i * 37}"})))
        store.execute("INSERT INTO orders VALUES (?, ?, 'delivered', ?, ?, ?, ?, 0, 0, ?, ?, ?, 'home')",
                      (oid, mid, delivered, delivered, json.dumps([{"item": item, "price": amount}]), amount, amount,
                       coupon, f"card_{1000 + i * 37}"))
        store.execute("INSERT INTO return_requests VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'refund', 'waiting')",
                      (f"ret_{i}", oid, mid, item, amount, reason, text, photos, delivered, "2026-11-05T18:00"))
    # The overcoat customer (#20431) already returned three things in the last 90 days.
    for j, at in enumerate(("2026-08-20T10:00", "2026-09-15T10:00", "2026-10-10T10:00"), start=1):
        store.execute("INSERT INTO return_requests VALUES (?, ?, 'mem_ret_20431', 'Earlier order', 120.0, 'changed_mind', "
                      "'Changed mind', 0, ?, ?, 'refund', 'refunded')", (f"ret_old_{j}", f"#1990{j}", at, at))
    store.execute("INSERT INTO meta (key, value) VALUES ('seq:ret', ?)", (str(len(RETURNS)),))


def seed_alex_return(store: Store) -> str:
    """For demos that skip scene 1: the request Alex would have sent from the storefront."""
    oid, mid, item, amount, reason, text, photos, delivered = ALEX_RETURN
    rid = store.new_id("ret")
    store.execute("INSERT INTO return_requests VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'refund', 'waiting')",
                  (rid, oid, mid, item, amount, reason, text, photos, delivered, store.now().isoformat(timespec="minutes")))
    return rid
