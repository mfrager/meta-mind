import { expect, test, type Locator } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route("http://127.0.0.1:8902/api/**", async (route) => {
    const url = new URL(route.request().url());
    const response = await route.fetch({
      url: `http://127.0.0.1:8894${url.pathname}${url.search}`,
    });
    await route.fulfill({ response });
  });
});

async function ask(pane: Locator, text: string) {
  const composer = pane.locator("#composer");
  await composer.fill(text);
  await composer.press("Control+Enter");
}

test("one session reads the guide, the state, a memory, and an empty change set", async ({ page }) => {
  await page.goto("/");
  const pane = page.locator("[data-chat-pane]");
  await expect(pane).toContainText("The kernel is");

  await ask(pane, "How do I use this?");
  await expect(pane).toContainText("Read the start of the guide.");

  await ask(pane, "Show me your state");
  await expect(pane).toContainText("This is the state.");
  await expect(page.locator("[data-state-board]")).toBeVisible();

  await ask(pane, "Remember that the migration order is fixed");
  await page.getByRole("button", { name: "Approve" }).click();
  await expect(pane).toContainText("Stored.");

  await ask(pane, "What did I just ask you to remember?");
  await expect(pane).toContainText("You asked me to remember that the migration order is fixed.");

  const approves = page.getByRole("button", { name: "Approve" });
  const parked = await approves.count();
  await ask(pane, "Promote something");
  await expect(pane).toContainText("No change sets are recorded.");
  await expect(approves).toHaveCount(parked);
});
