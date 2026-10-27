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
