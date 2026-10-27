import { expect, test } from "@playwright/test";
import { API } from "../playwright.config";

// Alex reports defective boots on the storefront and sends a refund request; the operator reviews 12 returns over $200
// and approves 9 refunds and 3 messages in one go.
test.beforeEach(async ({ request }) => {
  await request.post(`${API}/demo/reset`);
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-06T09:30" } });
});

test("Alex's request joins the operator's review, and one approval refunds 9", async ({ page, request }) => {
  await page.goto("/fo?shopper=alex");
  await page.getByRole("button", { name: /The sole on my chelsea boots/ }).click();
  const options = page.getByTestId("return_options");
  await expect(options).toContainText("Order #20402");
  await expect(options).toContainText("New pair, same size");
  await expect(options).toContainText("Today, 6 in stock");
  await expect(options).toContainText("$312.00 to card ending 0937");

  await page.getByPlaceholder("Ask about fit, fabric, delivery, returns…").fill("Refund, please. I'll pick a different pair later.");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Nothing is sent until you do.")).toBeVisible();
  await options.getByRole("button", { name: "Send refund request" }).click();
  await expect(page.getByTestId("return-sent")).toContainText("A person at Northwind confirms refunds over $200");

  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-06T10:40" } });
  await page.goto("/");
  await page.getByRole("button", { name: "Check returns waiting over $200" }).click();
  const review = page.getByTestId("refund-review");
  await expect(review).toContainText("12 requests");
  await expect(review).toContainText("9 · $2,463.00");
  await expect(review.getByRole("row", { name: /#20402/ })).toContainText("Defects are refunded in full");
  await expect(review.getByRole("row", { name: /#20290/ })).toContainText("Decline");
  await expect(review).toContainText("One customer has 4 returns in 90 days (#20431)");
  await expect(review).toContainText("1 to look at");

  await review.getByRole("button", { name: "Approve 9 refunds and send 3 messages" }).click();
  await expect(review).toContainText("Done · verified");
  await expect(page.getByTestId("verification")).toContainText("#20402 refunded $312.00");
  await expect(page.getByTestId("verification")).toContainText("Refunds are final and cannot be undone");
});
