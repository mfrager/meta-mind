import { expect, test } from "@playwright/test";

test("a scripted turn fills the dock without resizing the chat pane", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "What should we work on?" })).toBeVisible();
  const pane = page.locator("[data-agent-chat-pane]");
  const before = await pane.boundingBox();

  const composer = pane.locator("#chat-composer textarea");
  await composer.fill("How do I use this?");
  await composer.press("Enter");
  await expect(pane).toContainText("Read the start of the guide.");

  const workspace = page.locator("[data-agent-workspace-pane]");
  const dock = workspace.locator("[data-llm-log-dock]");
  await expect(dock).toContainText("LLM activity");
  await expect(page.locator("[data-agent-chat-pane] [data-llm-log-dock]")).toHaveCount(0);
  await expect(dock).toHaveAttribute("data-connection", "live");
  await expect(dock.locator("[data-kind='request']").first()).toBeVisible();
  await expect(dock.locator("[data-kind='response']").first()).toBeVisible();

  const kinds = await dock
    .locator("[data-kind]")
    .evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-kind")));
  const firstRequest = kinds.indexOf("request");
  const firstResponse = kinds.indexOf("response");
  expect(firstRequest).toBeGreaterThanOrEqual(0);
  expect(firstResponse).toBeGreaterThan(firstRequest);

  const dockHeight = (await dock.boundingBox())?.height ?? 0;
  await dock.getByRole("separator", { name: "Resize LLM activity" }).focus();
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await dock.boundingBox())?.height ?? 0).toBeGreaterThan(dockHeight);

  await page.keyboard.press("Control+`");
  await expect(dock).toHaveAttribute("data-collapsed", "true");
  await page.keyboard.press("Control+`");
  await expect(dock).toHaveAttribute("data-collapsed", "false");

  const after = await page.locator("[data-agent-chat-pane]").boundingBox();
  expect(Math.round(after?.height ?? 0)).toBe(Math.round(before?.height ?? 0));
});
