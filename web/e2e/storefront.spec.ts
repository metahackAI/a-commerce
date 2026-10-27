import { expect, test } from "@playwright/test";
import { API } from "../playwright.config";

// The storefront half of the demo: what the operator approved is what Jamie pays; Mina changes the layout by asking.
test.beforeEach(async ({ request }) => {
  await request.post(`${API}/demo/reset`);
});

async function approveVipWeekend(request: import("@playwright/test").APIRequestContext) {
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-06T11:02" } });
  const first = await (await request.post(`${API}/bo/chat`, { data: { request: "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page." } })).json();
  await request.post(`${API}/bo/chat`, { data: { request: "Drop the low-stock ones. Give the overshirt 15% instead.", change_set_id: first.change_set_id } });
  const live = await (await request.post(`${API}/change-sets/${first.change_set_id}/approve`, { data: { version: 2 } })).json();
  expect(live.status).toBe("live");
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-07T10:00" } });
}

async function say(page: import("@playwright/test").Page, text: string) {
  await page.getByPlaceholder("Ask about fit, fabric, delivery, returns…").fill(text);
  await page.keyboard.press("Enter");
}

test("Jamie compares, is remembered, and places the order at the VIP price", async ({ page, request }) => {
  await approveVipWeekend(request);
  await page.goto("/fo?shopper=jamie");

  await page.getByRole("button", { name: /I need a light jacket for spring trips/ }).click();
  const table = page.getByTestId("compare_table");
  await expect(table).toContainText("Shown as a table because you compare specs");
  await expect(table.getByRole("row", { name: /Your price/ })).toContainText("$71.20");
  await expect(table.getByRole("row", { name: /Your price/ })).toContainText("$67.15");
  await expect(page.getByTestId("chips")).toContainText("Size M");

  await say(page, "Which one packs smallest? I hate bulky bags.");
  await expect(page.getByTestId("memory")).toContainText("Remembered: weight and packed size matter to you when traveling");
  await expect(page.getByTestId("profile")).toContainText("Weight and packed size matter to you when traveling");

  await say(page, "The windbreaker. Same size as last time.");
  const cart = page.getByTestId("cart");
  await expect(cart).toContainText("Slate · M");
  await expect(cart).toContainText("VIP weekend");
  await expect(cart).toContainText("−$17.80");
  await expect(cart).toContainText("Card ending 4821");
  await cart.getByRole("button", { name: "Place order · $71.20" }).click();
  await expect(page.getByTestId("placed")).toHaveText("Order #20432 placed");
});

test("Mina asks for bigger photos, keeps the same results, and can undo the memory", async ({ page }) => {
  await page.goto("/fo?shopper=mina");
  await page.getByRole("button", { name: /I need a light jacket for spring trips/ }).click();
  const list = page.getByTestId("product_list");
  await expect(list).toContainText("Packable windbreaker");
  await expect(list).toContainText("$89.00");

  await say(page, "Less text. Just show me bigger photos.");
  const gallery = page.getByTestId("look_gallery");
  await expect(gallery).toContainText("Packable windbreaker");
  await expect(gallery).toContainText("Seersucker overshirt");
  const memory = page.getByTestId("memory");
  await expect(memory).toContainText("Remembered: photos over details");
  await memory.getByRole("button", { name: "Undo" }).click();
  await expect(memory).toContainText("Forgotten");
  await expect(page.getByTestId("profile")).toContainText("Nothing yet.");
});

test("a gift for Dad: the VIP price, the size, an address used once, nothing saved", async ({ page, request }) => {
  await approveVipWeekend(request);
  await page.goto("/fo?shopper=jamie");
  await say(page, "A gift for my dad. He hikes a lot. Under $100, and it has to arrive by Nov 14.");
  const gift = page.getByTestId("gift_picker");
  await expect(gift).toContainText("Packable windbreaker");
  await expect(gift).toContainText("$71.20");
  await expect(gift).toContainText("Gift receipt, no prices shown");
  await expect(gift.getByRole("button", { name: "Tell me the size first" })).toBeDisabled();
  await say(page, "He's about my size, maybe a bit bigger.");
  await expect(gift).toContainText("Chosen · L");
  await gift.getByPlaceholder("Who it is for").fill("Dad");
  await gift.getByPlaceholder("Their address").fill("48 Pine Rd, Lakeview");
  await gift.getByRole("button", { name: "Place gift order · $76.20" }).click();
  await expect(page.getByTestId("gift-placed")).toBeVisible();
  await expect(page.getByTestId("profile")).not.toContainText("Pine");
});

test("fit check compares the overcoat with the windbreaker Jamie kept", async ({ page, request }) => {
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-07T10:00" } });
  const h = { "X-Shopper": "jamie" };
  await request.post(`${API}/tools/add_to_cart`, { headers: h, data: { args: { product_id: "prd_windbreaker", size: "M" } } });
  const order = await (await request.post(`${API}/tools/prepare_order`, { headers: h, data: { args: {} } })).json();
  await request.post(`${API}/orders`, { headers: h, data: { order_preview_id: order.order_preview_id } });
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-20T10:00" } });

  await page.goto("/fo?shopper=jamie");
  await say(page, "Will M fit in the wool overcoat? I want to wear it over a hoodie.");
  const fit = page.getByTestId("fit_check");
  await expect(fit).toContainText("Over a hoodie");
  await expect(fit).toContainText("3 cm more in the chest than M");
  await expect(fit).toContainText("Your windbreaker, M");
  await expect(fit).toContainText("Not used: Height and weight");
  await expect(page.getByTestId("memory")).toContainText("Remembered: you layer hoodies under coats");
  await fit.getByRole("button", { name: "Add L · $289.00" }).click();
  await expect(page.getByTestId("cart")).toContainText("Charcoal · L");
  await page.getByTestId("cart").getByRole("button", { name: "Place order · $289.00" }).click();
  await expect(page.getByTestId("placed")).toBeVisible();
});

test("on a phone, Mina gets the same flow and the page never scrolls sideways", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/fo?shopper=mina");
  const noSideScroll = async () =>
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true);
  await noSideScroll();
  await page.getByRole("button", { name: /I need a light jacket for spring trips/ }).click();
  await expect(page.getByTestId("product_list")).toContainText("Packable windbreaker");
  await noSideScroll();
  await say(page, "Less text. Just show me bigger photos.");
  await expect(page.getByTestId("look_gallery")).toBeVisible();
  await expect(page.getByTestId("memory")).toContainText("Remembered: photos over details");
  await noSideScroll();
  await say(page, "Compare them.");
  await expect(page.getByTestId("compare_table")).toBeVisible();
  await noSideScroll(); // the table scrolls inside its box, not the page
});
