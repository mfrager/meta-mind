import { expect, test } from "@playwright/test";

test("shell reports a healthy kernel and opens the command palette", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-kernel-state]")).toHaveAttribute("data-kernel-state", "ok");
  await page.keyboard.press("Control+k");
  await expect(page.locator("[cmdk-dialog]")).toBeVisible();
});

test("budget show renders the ledger, not an invented run", async ({ page }) => {
  await page.goto("/chat");
  const pane = page.locator("[data-chat-pane]");
  const composer = pane.locator("#composer");
  await composer.fill("budget show");
  await expect(pane.getByRole("button", { name: "Send" })).toBeEnabled();
  await composer.press("Control+Enter");
  await expect(pane).toContainText("meta_analysis");
  await expect(pane).toContainText("Open the budgets tree");
});

test("reduced motion and higher contrast follow the system preferences", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  const fast = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--t-fast").trim());
  expect(fast).toBe("0ms");

  await page.emulateMedia({ contrast: "more" });
  const borders = await page.evaluate(() => {
    const style = getComputedStyle(document.documentElement);
    return {
      border: style.getPropertyValue("--border").trim(),
      strong: style.getPropertyValue("--border-strong").trim(),
    };
  });
  expect(borders.border).toBe(borders.strong);
});

test("catalog, settings, and the being map load as their own routes", async ({ page }) => {
  await page.goto("/dev/catalog");
  await expect(page.locator("[data-catalog-count]")).toHaveAttribute("data-catalog-count", "130");
  await expect(page.locator("[data-parity-count]")).toHaveAttribute("data-parity-count", "42");
  await page.goto("/settings");
  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await page.goto("/map/being");
  await expect(page.locator("body")).toContainText("This tree is a fixture. Live being records are not on this route yet.");
});

test("print hides the shell chrome", async ({ page }) => {
  await page.goto("/");
  await page.emulateMedia({ media: "print" });
  const display = await page.locator("[data-sidebar-panel]").evaluate((node) => getComputedStyle(node).display);
  expect(display).toBe("none");
});
