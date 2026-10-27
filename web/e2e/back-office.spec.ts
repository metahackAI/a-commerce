import { expect, test } from "@playwright/test";
import { API } from "../playwright.config";

// The demo's back office story in a real browser: words → change set v1 → v2 → approve → read back → undo.
test.beforeEach(async ({ request }) => {
  await request.post(`${API}/demo/reset`);
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-06T11:02" } });
});

test("operator builds, approves, verifies and undoes the VIP weekend sale", async ({ page, request }) => {
  await page.goto("/");
  const cs = page.getByTestId("change-set");

  await page.getByRole("button", { name: /Run a 20% weekend sale/ }).click();
  await expect(cs.getByText("#1042 · v1")).toBeVisible();
  await expect(cs.getByText("4 of 6 passed")).toBeVisible();
  await expect(cs.getByText("Seersucker overshirt would be $63.20, floor $64.40")).toBeVisible();
  await expect(cs.getByRole("button", { name: "Approve and run" })).toBeDisabled();
  await expect(page.getByText("Two things need your call")).toBeVisible();

  await page.getByPlaceholder("Ask for changes to this set…").fill("Drop the low-stock ones. Give the overshirt 15% instead.");
  await page.keyboard.press("Enter");
  await expect(cs.getByText("#1042 · v2")).toBeVisible();
  await expect(cs.getByText("6 of 6 passed")).toBeVisible();
  await expect(cs.getByText("40 of 42 · 2 excluded")).toBeVisible();
  await expect(cs.getByRole("row", { name: /Seersucker overshirt/ })).toContainText("$67.15");
  await expect(cs.getByRole("row", { name: /Packable windbreaker/ })).toContainText("$71.20");
  await expect(cs.getByText("1,284 VIP members")).toBeVisible();

  await cs.getByRole("button", { name: "Approve and run" }).click();
  const status = cs.locator(".csh .tag");
  await expect(status).toHaveText("Live · verified");
  const readBack = page.getByTestId("verification");
  await expect(readBack).toContainText("Everything matches this change set");
  await expect(readBack).toContainText("40 of 40 sale prices match");
  await expect(readBack).toContainText("VIPWEEKEND20 issued to 1,284 members");

  // What a VIP shopper pays on Saturday, through the same tools.
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-07T10:00" } });
  const price = async () =>
    (await (await request.post(`${API}/tools/compare_products`, {
      headers: { "X-Shopper": "jamie" },
      data: { args: { product_ids: ["prd_windbreaker", "prd_overshirt"], attributes: ["price"] } },
    })).json()).rows[0].values.map((v: { now: number }) => v.now);
  expect(await price()).toEqual([71.2, 67.15]);

  await cs.getByRole("button", { name: "Undo all together" }).click();
  await expect(status).toHaveText("Undone");
  expect(await price()).toEqual([89, 79]);
});

test("a reply that contradicts the change set is replaced and the correction is shown", async ({ page, request }) => {
  // The model's words come from a real Llama 3.2 3B run; the change set comes from the real API.
  await page.route("**/api/bo/chat", async (route) => {
    const real = await (await route.fetch()).json();
    real.events[0].text = "Change set #1042 v1 is ready for review. Nothing has changed in the store yet.";
    real.events[0].corrected = {
      said: "A 20% weekend sale on summer outerwear is now live for VIP members on the main page.",
      issues: ["It said the change is live, but nothing changes until you approve."],
    };
    await route.fulfill({ json: real });
  });
  await page.goto("/");
  await page.getByRole("button", { name: /Run a 20% weekend sale/ }).click();
  const note = page.getByTestId("corrected");
  await expect(note).toContainText("Reply checked against the change set: 1 correction");
  await note.locator("summary").click();
  await expect(note).toContainText("It said the change is live, but nothing changes until you approve.");
  await expect(page.getByText("Nothing has changed in the store yet.")).toBeVisible();
  expect((await (await request.get(`${API}/change-sets/1042`)).json()).status).toBe("waiting_for_approval");
});

test("home is a task list: open the sale from it, approve, and see it move to Done", async ({ page }) => {
  await page.goto("/");
  const home = page.getByTestId("home");
  await expect(home.getByRole("button", { name: /11 return requests over \$200/ })).toBeVisible();
  await expect(home.getByRole("button", { name: /Linen shirts will sell out/ })).toBeDisabled(); // seeded example

  await home.getByRole("button", { name: /Run a 20% weekend sale/ }).click();
  await page.getByPlaceholder("Ask for changes to this set…").fill("Drop the low-stock ones. Give the overshirt 15% instead.");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("change-set").getByText("#1042 · v2")).toBeVisible();

  await page.getByRole("button", { name: "Back to tasks" }).click();
  const sale = home.getByTestId("task").filter({ hasText: "VIP weekend sale: summer outerwear" });
  await expect(sale).toContainText("Ready to review");
  await expect(sale).toContainText("~40 prices");
  await expect(sale).toContainText("6/6 checks");
  await sale.click();

  const cs = page.getByTestId("change-set");
  await expect(cs.getByText("#1042 · v2")).toBeVisible();
  await cs.getByRole("button", { name: "Approve and run" }).click();
  await expect(cs.locator(".csh .tag")).toHaveText("Live · verified");

  await page.getByRole("button", { name: "Back to tasks" }).click();
  await home.getByRole("button", { name: /^Done/ }).click();
  await expect(home.getByTestId("task").filter({ hasText: "VIP weekend sale" })).toContainText("Live · verified");
});

test("a question gets a read-only answer whose numbers add up", async ({ page, request }) => {
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-06T11:20" } });
  await page.goto("/");
  await page.getByRole("button", { name: "Why did sales drop yesterday?" }).click();
  const answer = page.getByTestId("sales-explainer");
  await expect(answer).toContainText("Nothing changed");
  await expect(answer).toContainText("$18,240.00");
  await expect(answer).toContainText("−$4,660.00 (-20%)");
  await expect(answer.getByRole("row", { name: /3 bestsellers out of M and L/ })).toContainText("−$2,700.00");
  await expect(answer.getByRole("row", { name: /Weekly email not sent/ })).toContainText("−$1,300.00");
  await expect(answer.getByRole("row", { name: /Not explained/ })).toContainText("−$660.00");
  await expect(answer.getByRole("button", { name: "Send the weekly email today" })).toBeVisible();
  await expect(page.getByText("Revenue was $18,240, 20% under a normal Thursday.")).toBeVisible();
  await expect(page.getByText("Read only · nothing changed")).toBeVisible();
  // Nothing was proposed.
  expect((await (await request.get(`${API}/change-sets`)).json()).length).toBe(0);
});

test("a next step from the answer becomes a change set: send the email, which cannot be undone", async ({ page, request }) => {
  await request.post(`${API}/demo/clock`, { data: { at: "2026-11-06T11:20" } });
  await page.goto("/");
  await page.getByRole("button", { name: "Why did sales drop yesterday?" }).click();
  await page.getByTestId("sales-explainer").getByRole("button", { name: "Send the weekly email today" }).click();
  const cs = page.getByTestId("change-set");
  await expect(cs).toContainText("Weekly email");
  await expect(cs).toContainText("Not possible once sent");
  await expect(cs).toContainText("Cannot be undone");
  await cs.getByRole("button", { name: "Approve and run" }).click();
  await expect(cs.locator(".csh .tag")).toHaveText("Live · verified");
  await expect(page.getByTestId("verification")).toContainText("Weekly email sent 2026-11-06");
  await expect(cs.getByRole("button", { name: "Undo all together" })).toHaveCount(0);
});

test("a supplier sheet becomes 26 drafts and 4 rows that need a person; saved drafts stay off the store", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "New products from fw26_supplier.xlsx" }).click();
  const drafts = page.getByTestId("product-drafts");
  await expect(drafts).toContainText("30 rows");
  await expect(drafts.getByRole("row", { name: /Wool blend overcoat, charcoal/ })).toContainText("$239.00");
  await expect(drafts.getByRole("row", { name: /Quilted liner jacket, olive/ })).toContainText("Fill % missing in the sheet");
  await expect(drafts.getByRole("row", { name: /Corduroy trousers, sand/ })).toContainText("Photo shows two items");
  await drafts.getByRole("button", { name: "Save 26 ready as drafts" }).click();
  await expect(drafts).toContainText("Saved · verified");
  await expect(page.getByTestId("verification")).toContainText("26 drafts saved, not visible to shoppers");
});
