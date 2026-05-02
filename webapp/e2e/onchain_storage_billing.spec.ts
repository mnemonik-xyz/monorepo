import { test, expect } from "@playwright/test";
import { mcpBase, randomHex, pkceChallenge } from "./_helpers";

test.describe.configure({ mode: "serial", retries: 0 });

test("onchain storage + billing (Phase 1.5)", async ({ page }) => {
  await page.goto(mcpBase + "/setup");

  // Set STORAGE_MODE to full
  await page.fill("input[name='STORAGE_MODE']", "full");
  await page.click("button[name='save-storage-mode']");

  // Wait for save confirmation
  await expect(page.locator("#storage-mode-status")).toHaveText("Saved", { timeout: 10000 });

  // Set PAYMENT_MODE to balance
  await page.fill("input[name='PAYMENT_MODE']", "balance");
  await page.click("button[name='save-payment-mode']");

  // Wait for save confirmation
  await expect(page.locator("#payment-mode-status")).toHaveText("Saved", { timeout: 10000 });

  // Verify both modes are now set correctly
  const storageModeValue = await page.inputValue("input[name='STORAGE_MODE']");
  const paymentModeValue = await page.inputValue("input[name='PAYMENT_MODE']");

  expect(storageModeValue).toBe("full");
  expect(paymentModeValue).toBe("balance");

  // Navigate to balance page
  await page.goto(mcpBase + "/balance");

  // Check for USDC balance display and top-up button
  await expect(page.locator("#usdc-balance")).toBeVisible();
  await expect(page.locator("#top-up-button")).toBeVisible();

  // Check for low balance warning if applicable
  const balanceText = await page.locator("#usdc-balance").textContent();
  const balance = parseFloat(balanceText?.replace(/[^0-9.-]+/g, "") || "0");
  if (balance < 0.1) {
    await expect(page.locator("#low-balance-warning")).toBeVisible();
  }

  // Perform a mnemonic sign memory flow to trigger on-chain write
  await page.goto(mcpBase + "/sign-flow");

  // Generate test content
  const testContent = `Test memory ${randomHex(8)}`;
  await page.fill("textarea[name='content']", testContent);
  await page.click("button[name='sign-memory']");

  // Wait for success confirmation with on-chain IDs
  const result = await page.locator("#sign-result").textContent();
  expect(result).toMatch(/arweave_tx:[a-z0-9]+/i);
  expect(result).toMatch(/solana_tx:[a-z0-9]+/i);

  // Verify the attestation can be recalled
  await page.goto(mcpBase + "/recall");
  await page.fill("input[name='query']", testContent);
  await page.click("button[name='recall-memory']");

  const recallResult = await page.locator("#recall-result").textContent();
  expect(recallResult).toContain(testContent);
});