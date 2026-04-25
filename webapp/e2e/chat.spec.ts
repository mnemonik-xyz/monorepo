import { test, expect, Page } from "@playwright/test";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Mock a successful /chat response. */
async function mockChatSuccess(page: Page, responseText: string) {
  await page.route("**/chat", (route) => {
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ response: responseText }),
    });
  });
}

/** Navigate to landing, click "Start chat", wait for chat view. */
async function navigateToChat(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Start chat" }).click();
  await expect(
    page.getByRole("heading", { name: "Protocol Chat" })
  ).toBeVisible();
}

/** Type a message and click Send. */
async function sendMessage(page: Page, text: string) {
  const input = page.getByLabel("Message input");
  await input.fill(text);
  await page.getByLabel("Send message").click();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe("Chat E2E", () => {
  test("golden path: open -> start chat -> send question -> receive answer -> download link", async ({
    page,
  }) => {
    const botAnswer =
      "Mnemonic Protocol provides verifiable, persistent memory for AI agents.";

    await mockChatSuccess(page, botAnswer);

    // Mock the download endpoint so the link click succeeds
    await page.route("**/api/download-knowledge", (route) => {
      route.fulfill({
        status: 200,
        contentType: "application/zip",
        body: Buffer.from("PK-fake-zip"),
        headers: {
          "Content-Disposition": 'attachment; filename="knowledge.zip"',
        },
      });
    });

    // Landing page is visible
    await page.goto("/");
    await expect(
      page.getByRole("heading", { name: "Mnemonic Protocol" })
    ).toBeVisible();

    // Download link exists on landing with correct href
    const downloadLink = page.getByRole("link", {
      name: "Download protocol knowledge",
    });
    await expect(downloadLink).toBeVisible();
    await expect(downloadLink).toHaveAttribute(
      "href",
      "/api/download-knowledge"
    );
    await expect(downloadLink).toHaveAttribute("download", "");

    // Verify download triggers (intercept the navigation/download)
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      downloadLink.click(),
    ]);
    expect(download.suggestedFilename()).toBe("knowledge.zip");

    // Navigate to chat
    await page.getByRole("button", { name: "Start chat" }).click();
    await expect(
      page.getByRole("heading", { name: "Protocol Chat" })
    ).toBeVisible();

    // Empty state prompt visible
    await expect(
      page.getByText("Ask a question about the Mnemonic Protocol.")
    ).toBeVisible();

    // Send a question
    await sendMessage(page, "What is Mnemonic Protocol?");

    // User message appears in the log region
    const messageLog = page.getByRole("log");
    await expect(
      messageLog.getByText("What is Mnemonic Protocol?")
    ).toBeVisible();

    // Bot answer appears in the log region
    await expect(messageLog.getByText(botAnswer)).toBeVisible();

    // Counter incremented to 1/50
    await expect(page.getByLabel("Session message counter")).toHaveText("1/50");
  });

  test("out-of-scope question: verify rejection message", async ({ page }) => {
    const rejectionMsg =
      "This question is outside the scope of the Mnemonic Protocol.";

    await mockChatSuccess(page, rejectionMsg);
    await navigateToChat(page);
    await sendMessage(page, "What is the weather today?");

    // User message visible
    await expect(page.getByText("What is the weather today?")).toBeVisible();

    // Bot rejection visible
    await expect(page.getByText(rejectionMsg)).toBeVisible();
  });

  test("session limit: counter at 49, send 1 message -> limit notification", async ({
    page,
  }) => {
    const botAnswer = "Here is your answer.";

    await mockChatSuccess(page, botAnswer);
    await navigateToChat(page);

    // Send 49 messages with instant mock responses to reach counter = 49.
    // Each iteration: fill input, click send, wait for the counter to update.
    for (let i = 1; i <= 49; i++) {
      await page.getByLabel("Message input").fill(`msg ${i}`);
      await page.getByLabel("Send message").click();
      await expect(page.getByLabel("Session message counter")).toHaveText(
        `${i}/50`
      );
    }

    // Counter should be 49/50
    await expect(page.getByLabel("Session message counter")).toHaveText(
      "49/50"
    );

    // Send the 50th message -- this should trigger the session limit
    await sendMessage(page, "final question");

    // Counter at 50/50
    await expect(page.getByLabel("Session message counter")).toHaveText(
      "50/50"
    );

    // Limit banner appears
    await expect(page.getByRole("alert")).toBeVisible();
    await expect(page.getByRole("alert")).toContainText(
      "Session limit reached"
    );

    // Input is disabled
    await expect(page.getByLabel("Message input")).toBeDisabled();
    await expect(page.getByLabel("Send message")).toBeDisabled();
  });

  test("error state: service unavailable -> verify error message after retries", async ({
    page,
  }) => {
    // Install fake timers so retry backoff (1s, 2s) resolves instantly.
    await page.clock.install();

    // Mock /chat to always return 503 (retryable). The client retries 3 times.
    let callCount = 0;
    await page.route("**/chat", (route) => {
      callCount++;
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({
          error: "Service unavailable",
          code: "service_unavailable",
        }),
      });
    });

    await navigateToChat(page);
    await sendMessage(page, "trigger error");

    // User message visible
    await expect(page.getByText("trigger error")).toBeVisible();

    // Advance clock past retry backoff delays (1s + 2s = 3s total)
    await page.clock.fastForward(5000);

    // Error message appears after retries exhaust (3 attempts with backoff).
    // The mapped message for service_unavailable is:
    // "Service temporarily unavailable. Try again later."
    await expect(
      page.getByText("Service temporarily unavailable. Try again later.")
    ).toBeVisible();

    // Verify retries happened (should be 3 attempts)
    expect(callCount).toBe(3);
  });

  test("error state: non-retryable 429 -> immediate error without retries", async ({
    page,
  }) => {
    let callCount = 0;
    await page.route("**/chat", (route) => {
      callCount++;
      route.fulfill({
        status: 429,
        contentType: "application/json",
        body: JSON.stringify({
          error: "Rate limited",
          code: "rate_limited",
        }),
      });
    });

    await navigateToChat(page);
    await sendMessage(page, "rate limit test");

    await expect(
      page.getByText(
        "Rate limit exceeded. Wait before sending another request."
      )
    ).toBeVisible();

    // Non-retryable: should be exactly 1 call
    expect(callCount).toBe(1);
  });

  test("back button returns to landing page", async ({ page }) => {
    await navigateToChat(page);
    await page.getByLabel("Back to landing page").click();
    await expect(
      page.getByRole("heading", { name: "Mnemonic Protocol" })
    ).toBeVisible();
  });
});
