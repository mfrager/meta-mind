import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route("http://127.0.0.1:8902/api/**", async (route) => {
    const url = new URL(route.request().url());
    const response = await route.fetch({
      url: `http://127.0.0.1:8895${url.pathname}${url.search}`,
    });
    await route.fulfill({ response });
  });
});

test("a scripted budget.show replaces the state board", async ({ page }) => {
  await page.goto("/");
  const pane = page.locator("[data-chat-pane]");
  await expect(pane).toContainText("The kernel is");
  const composer = pane.locator("#composer");
  await composer.fill("what are the budgets?");
  await composer.press("Control+Enter");
  await expect(page.locator("[data-doc-kind='budget']")).toContainText("meta_analysis");
  await expect(page.locator("[data-state-board]")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Approve" })).toHaveCount(0);
});
