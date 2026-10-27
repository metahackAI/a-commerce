# UI mockups (Oct 27)

Static HTML. Open the files in a browser; the PNGs are 1440×900 captures. The same layout serves the web app and the desktop app.

| Screen | File | What it shows |
|---|---|---|
| Back office home | `home.html` | Task input, items waiting for the operator, what the agent can do in the store |
| Back office task | `bo.html` (`bo.png`, `bo-2.png`) | One request becomes tool calls, a product table, an approval card, and a storefront preview with the list of changes |
| Storefront | `fo.html` (`fo.png`, `fo-2.png`) | A shopper conversation: gallery, then a comparison table because this shopper compares specs, then a cart with the VIP coupon from the back-office task |

Layout: conversations on the left, the thread with UI blocks in the middle, and a side panel on the right (preview, changes, log, or cart). Block names in the corner (`product_table`, `approval_card`, …) match the UI protocol in `docs/design.md`.

All store, product, and person names are made up.
