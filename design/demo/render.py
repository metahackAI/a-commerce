# Render each storyboard frame to frames/<id>.png (1920x1080), the contact sheet overview.png, and
# the README's architecture picture (docs/img/ai-native-architecture.png, frame 13 without what is
# not running yet, the story's moments and the subtitle). Needs Python Playwright with Chromium (pip install playwright && playwright install chromium) and
# Pretendard installed locally, as the app uses it. Usage: python design/demo/render.py
import asyncio, html, pathlib, sys, tempfile
from playwright.async_api import async_playwright

ROOT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else __file__).resolve()
ROOT = ROOT if ROOT.is_dir() else ROOT.parent
OUT = ROOT / "frames"

SHEET = """<!doctype html><html><head><meta charset="utf-8"><style>
body {{ margin: 0; background: #fff; font-family: "Pretendard Variable", Pretendard, system-ui, sans-serif; color: #000; }}
.wrap {{ width: 1872px; padding: 28px 24px 24px; }}
h1 {{ font-size: 22px; font-weight: 700; margin: 0 0 4px; letter-spacing: -.01em; }}
.sub {{ font-size: 14px; color: #474747; margin-bottom: 20px; }}
.grid {{ display: grid; grid-template-columns: repeat(5, 360px); gap: 22px 18px; }}
img {{ width: 360px; height: 202.5px; display: block; border: 1px solid #e6e6e6; border-radius: 6px; }}
.cap {{ margin-top: 7px; font-size: 13px; line-height: 1.35; }}
.cap b {{ font-variant-numeric: tabular-nums; margin-right: 6px; }}
.cap span {{ color: #707070; margin-right: 6px; font-variant-numeric: tabular-nums; }}
</style></head><body><div class="wrap">
<h1>A-Commerce demo video · storyboard</h1>
<div class="sub">15 key frames, 2:50 · numbers are illustrative until the Nov 3 recording · full notes in storyboard.html</div>
<div class="grid">{cells}</div></div></body></html>"""


async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        # one frame fills the 1440x810 viewport; at 4/3 scale that is exactly 1920x1080
        page = await (await browser.new_context(viewport={"width": 1440, "height": 810}, device_scale_factor=4 / 3)).new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        await page.goto((ROOT / "storyboard.html").as_uri(), wait_until="networkidle")
        await page.wait_for_timeout(500)
        # a subtitle should stay on one line inside the frame
        for sid, problem in await page.evaluate("""() => [...document.querySelectorAll('.frame')].flatMap(f => {
            const fr = f.getBoundingClientRect(), id = f.closest('section').id;
            return [...f.querySelectorAll('.cap')].flatMap(c => { const r = c.getBoundingClientRect();
              return r.height > 50 ? [[id, 'subtitle wraps']] : (r.left < fr.left || r.right > fr.right) ? [[id, 'subtitle outside']] : []; });
          })"""):
            print(sid, problem)
        # show one frame at a time at the top left, with no page around it
        hide = """
          body, .page, .board { margin: 0 !important; padding: 0 !important; max-width: none !important; }
          .page > header, .board > .meta, .board > .notes, .board:not(.shoot) { display: none !important; }
          .frame { outline: none !important; }"""
        await page.add_style_tag(content=hide)
        OUT.mkdir(exist_ok=True)

        async def shoot(sid):
            await page.evaluate("id => document.querySelectorAll('section.board').forEach(b => b.classList.toggle('shoot', b.id === id))", sid)
            await page.screenshot(path=str(OUT / f"{sid}.png"))

        boards = [await b.get_attribute("id") for b in await page.query_selector_all("section.board")]
        for sid in boards:
            await shoot(sid)
        cells = []
        for sid in boards:
            board = await page.query_selector(f"#{sid}")
            tc = await board.eval_on_selector(".meta .tc", "e => e.textContent")
            name = await board.eval_on_selector(".meta .name", "e => e.textContent")
            cells.append(f'<div><img src="{(OUT / f"{sid}.png").as_uri()}"><div class="cap"><b>{sid[1:]}</b>'
                         f'<span>{html.escape(tc)}</span>{html.escape(name)}</div></div>')
        # the README picture: what runs today, nothing from the video around it
        arch = page.locator("#arch")
        if await arch.count():
            await page.evaluate("() => document.querySelectorAll('section.board').forEach(b => b.classList.toggle('shoot', b.querySelector('#arch') !== null))")
            await page.add_style_tag(content=".planned, .arch .seen, .cap { display: none !important; }")
            readme_img = ROOT.parent.parent / "docs" / "img" / "ai-native-architecture.png"
            readme_img.parent.mkdir(parents=True, exist_ok=True)
            box = await arch.bounding_box()
            pad = 16  # keep the outer borders inside the picture
            await page.screenshot(path=str(readme_img), clip={"x": box["x"] - pad, "y": box["y"] - pad,
                                                              "width": box["width"] + 2 * pad, "height": box["height"] + 2 * pad})
        with tempfile.TemporaryDirectory() as tmp:
            sheet = pathlib.Path(tmp) / "overview.html"
            sheet.write_text(SHEET.format(cells="".join(cells)), encoding="utf-8")
            p2 = await (await browser.new_context(viewport={"width": 1920, "height": 900}, device_scale_factor=1.5)).new_page()
            await p2.goto(sheet.as_uri(), wait_until="networkidle")
            await p2.locator(".wrap").screenshot(path=str(ROOT / "overview.png"))
        await browser.close()
        print(f"{len(cells)} frames", *errors, sep="\n")

asyncio.run(main())
