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
