import { expect, test } from "@playwright/test";
import { API } from "../playwright.config";

// The README walkthrough, in order, through the UI only (no API shortcuts after the reset).
test("the README demo walkthrough works start to finish", async ({ page, request }) => {
  test.setTimeout(60_000);
  await request.post(`${API}/demo/reset`);
  const say = async (text: string) => {
    await page.getByPlaceholder(/Ask about fit|Ask for changes|Describe a task/).fill(text);
    await page.keyboard.press("Enter");
  };

  // 1. Alex
  await page.goto("/fo?shopper=alex");
  await page.getByRole("button", { name: /The sole on my chelsea boots/ }).click();
  await say("Refund, please.");
  await page.getByRole("button", { name: "Send refund request" }).click();
  await expect(page.getByTestId("return-sent")).toBeVisible();

  // 2. Returns review
  await page.goto("/");
  await page.getByRole("button", { name: "Check returns waiting over $200" }).click();
  const review = page.getByTestId("refund-review");
  await expect(review).toContainText("9 · $2,463.00");
  await review.getByRole("button", { name: "Approve 9 refunds and send 3 messages" }).click();
  await expect(review).toContainText("Done · verified");

  // 3. VIP weekend sale
  await page.goto("/");
  await page.getByRole("button", { name: "Fri Nov 6, 11:02" }).click();
  await page.getByRole("button", { name: /Run a 20% weekend sale/ }).click();
  const cs = page.getByTestId("change-set");
  await expect(cs.getByText("#1042 · v1")).toBeVisible();
  await say("Drop the low-stock ones. Give the overshirt 15% instead.");
  await expect(cs.getByText("6 of 6 passed")).toBeVisible();
  await cs.getByRole("button", { name: "Approve and run" }).click();
  await expect(cs.locator(".csh .tag")).toHaveText("Live · verified");

  // 4. Jamie on Saturday
  await page.getByRole("button", { name: "Sat Nov 7, 10:00" }).click();
  await page.getByRole("link", { name: "Open storefront →" }).click();
  await page.getByRole("button", { name: /I need a light jacket for spring trips/ }).click();
  await expect(page.getByTestId("compare_table")).toContainText("$71.20");
  await say("Which one packs smallest? I hate bulky bags.");
  await expect(page.getByTestId("memory")).toBeVisible();
  await say("The windbreaker. Same size as last time.");
  await page.getByRole("button", { name: "Place order · $71.20" }).click();
  await expect(page.getByTestId("placed")).toBeVisible();

  // 5. Mina
  await page.getByRole("link", { name: "Mina · first visit" }).click();
  await page.getByRole("button", { name: /I need a light jacket for spring trips/ }).click();
  await expect(page.getByTestId("product_list")).toBeVisible();
  await say("Less text. Just show me bigger photos.");
  await expect(page.getByTestId("look_gallery")).toBeVisible();
});
