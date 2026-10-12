import { expect, test } from "@playwright/test";

test("a fresh session shows the intro, the composer, and the state board", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-kernel-state]")).toHaveAttribute("data-kernel-state", "ok");
  await expect(page.getByRole("navigation", { name: "Menu" })).toHaveCount(0);
  const pane = page.locator("[data-chat-pane]");
  await expect(pane).toContainText("The kernel is");
  await expect(pane.locator("#composer")).toHaveAttribute("placeholder", "Ask Metamind…");
  await expect(page.locator("[data-kernel-line]")).toContainText("kernel ok");
  await page.keyboard.press("Control+k");
  await expect(pane.locator("#composer")).toBeFocused();
  await pane.locator("#composer").fill("not sent");
  await page.keyboard.press("Escape");
  await expect(pane.locator("#composer")).toHaveValue("not sent");
  await expect(page.getByRole("button", { name: "Approve" })).toHaveCount(0);
});

test("the controller introduces itself before the operator types", async ({ page }) => {
  await page.goto("/chat");
  const pane = page.locator("[data-chat-pane]");
  await expect(pane).toContainText("The kernel is");
  await expect(pane.getByRole("button", { name: "How do I use this?" })).toBeVisible();
  const composer = pane.locator("#composer");
  await composer.fill("hello");
  await expect(pane.getByRole("button", { name: "Send" })).toBeEnabled();
  await composer.press("Control+Enter");
  await expect(pane).toContainText("I can't call a model yet.");
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

test("print hides the composer and keeps the transcript and the side pane", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-chat-pane]")).toContainText("The kernel is");
  await page.emulateMedia({ media: "print" });
  const composer = await page.locator("[data-composer-form]").evaluate((node) => getComputedStyle(node).display);
  const side = await page.locator("[data-side-pane]").evaluate((node) => getComputedStyle(node).display);
  const transcript = await page.locator("[data-chat-pane]").evaluate((node) => getComputedStyle(node).display);
  expect(composer).toBe("none");
  expect(side).not.toBe("none");
  expect(transcript).not.toBe("none");
});
