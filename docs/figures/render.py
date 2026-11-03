"""Renders the architecture figures to docs/img/ (Python Playwright with Chromium).

    python docs/figures/render.py
"""
from pathlib import Path

from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "img"
FIGURES = {"components.html": "fig-harness-components.png", "loops.html": "fig-two-loops.png"}

with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 1800, "height": 1200}, device_scale_factor=2)
    for src, png in FIGURES.items():
        if not (HERE / src).exists():
            continue
        page.goto((HERE / src).as_uri())
        page.wait_for_load_state("networkidle")
        page.evaluate("document.fonts.ready")
        page.locator(".fig").screenshot(path=str(OUT / png))
        print(OUT / png)
    browser.close()
