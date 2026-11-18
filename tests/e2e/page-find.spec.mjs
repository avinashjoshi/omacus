import { test, expect } from '@playwright/test';

async function open(page) {
  await page.goto('/app/');
  await page.locator('.cm-content').waitFor();
  await page.locator('.cm-content').click();
  await page.keyboard.press('Control+a');
  await page.keyboard.insertText('🍎 Café\ncAFÉ + café\na.b a+b');
  await page.keyboard.press('Control+f');
}
test('find highlights literal Unicode text, wraps, and closes without editing', async ({page}) => {
  await open(page);
  const input = page.locator('[data-page-find-input]');
  await expect(input).toBeFocused();
  await input.fill('café');
  await expect(page.locator('[data-page-find-count]')).toHaveText('1 / 3');
  await expect.poll(async () => (await page.locator('.page-match').allTextContents()).join('')).toBe('CafécAFÉcafé');
  await input.press('Shift+Enter');
  await expect(page.locator('[data-page-find-count]')).toHaveText('3 / 3');
  await input.press('Enter');
  await expect(page.locator('[data-page-find-count]')).toHaveText('1 / 3');
  await page.getByRole('button', {name:'Next match', exact:true}).click();
  await expect(page.locator('[data-page-find-count]')).toHaveText('2 / 3');
  await input.fill('.');
  await expect(page.locator('[data-page-find-count]')).toHaveText('1 / 1');
  await input.fill('missing');
  await expect(page.locator('[data-page-find-count]')).toHaveText('No matches');
  await expect(page.getByRole('button', {name:'Next match', exact:true})).toBeDisabled();
  await input.fill('');
  await expect(page.locator('.page-match')).toHaveCount(0);
  await input.press('Escape');
  await expect(page.locator('[data-page-find]')).toBeHidden();
  await expect(page.locator('.cm-content')).toBeFocused();
  await expect(page.locator('.cm-content')).toContainText('🍎 Café');
  await page.keyboard.press('Control+j');
  await expect(page.locator('[data-switcher]')).toBeVisible();
});
test('find follows edits and page changes, and scrolls to distant matches', async ({page}) => {
  await open(page);
  await page.locator('[data-page-find-input]').fill('café');
  await page.locator('.cm-content').click();
  await page.keyboard.press('Control+a');
  await page.keyboard.insertText(Array.from({length:100}, (_,i)=>`Line ${i}`).join('\n')+'\nCafé');
  await expect(page.locator('[data-page-find-count]')).toHaveText('1 / 1');
  await page.getByRole('button', {name:'Next match', exact:true}).click();
  await expect(page.locator('.page-match-current').first()).toBeInViewport();
  await page.locator('[data-id="ex1"] .side-name').click();
  await expect(page.locator('[data-page-find-count]')).toHaveText('No matches');
});

test('touch layout exposes find through the page menu', async ({browser}) => {
  const context = await browser.newContext({ viewport: {width:390,height:844}, hasTouch:true, isMobile:true });
  const page = await context.newPage();
  await page.goto('/app/');
  await page.locator('.cm-content').waitFor();
  await page.locator('[data-menu-button]').click();
  await page.getByRole('menuitem', {name:'Find in page', exact:true}).click();
  await expect(page.locator('[data-page-find-input]')).toBeFocused();
  await page.locator('[data-page-find-input]').fill('page');
  await expect(page.locator('.page-match').first()).toBeVisible();
  const bar = await page.locator('[data-page-find]').boundingBox();
  expect(bar.x + bar.width).toBeLessThanOrEqual(390);
  await page.getByRole('button', {name:'Close find', exact:true}).click();
  await expect(page.locator('[data-page-find]')).toBeHidden();
  await context.close();
});
