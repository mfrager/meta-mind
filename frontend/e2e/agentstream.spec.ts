import { expect, test } from "@playwright/test";

test("a mock action turn shows the transcript, full prompt, tools, and judge result", async ({ page }) => {
  await page.goto("/chat/s-demo");

  await page.getByLabel("Message").fill("Please set the topic to incident");
  await page.getByRole("button", { name: "Send" }).click();

  const transcript = page.getByRole("log", { name: "Transcript" });
  await expect(transcript.getByText("The topic is now incident.")).toBeVisible();

  const processCall = page.locator('[data-dock-row][data-event-type="llm.request"]').nth(1);
  await processCall.getByRole("button").first().click();

  const viewer = page.locator("[data-call-viewer]");
  await expect(viewer.getByText("You are Metamind. FULL SYSTEM PROMPT for process.")).toBeVisible();
  await expect(viewer.getByText(/\[STATE\]/)).toBeVisible();

  await viewer.getByRole("tab", { name: "Tools" }).click();
  const offered = viewer.locator("[data-viewer-tools]");
  await expect(offered.getByText("conversation.set_topic")).toBeVisible();
  await expect(offered.getByText("conversation.set_depth")).toBeVisible();
  await expect(offered.getByText("metamind.verify")).toBeVisible();
  await expect(offered.getByText("metamind.approve")).toBeVisible();
  await expect(offered.getByText("Set the conversation topic.")).toBeVisible();

  await viewer.getByRole("tab", { name: "Response" }).click();
  await expect(viewer.locator("[data-viewer-response]").getByText("Setting the topic.")).toBeVisible();

  await viewer.getByRole("tab", { name: "Internals" }).click();
  await expect(viewer.locator("[data-viewer-internals]").getByText(/process-hash/)).toBeVisible();
  await expect(viewer.locator("[data-viewer-internals]").getByText(/The staged state sets topic to incident/)).toBeVisible();

  // The same rows come back from the persisted snapshot after a reload.
  await page.reload();
  await expect(transcript.getByText("The topic is now incident.")).toBeVisible();
  await expect(page.locator('[data-dock-row][data-event-type="verification.result"]')).toBeVisible();
});
