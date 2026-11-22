import { join } from 'node:path';
import { expect, test as base, type Page } from '@playwright/test';

export type Role = 'admin' | 'member';

export type Account = { username: string; password: string };

/** The local accounts `corral.ts` made, readable only inside a test. */
export function account(role: Role): Account {
	const accounts: Record<string, Account> = JSON.parse(process.env.CORRAL_E2E_ACCOUNTS ?? '{}');
	return accounts[role];
}

/** Where `corral.ts` keeps a signed-in session of each account. */
export function session(role: Role): string {
	return join(import.meta.dirname, '..', 'playwright', '.auth', `${role}.json`);
}

/** Fills in the sign-in page's password form. */
export async function signIn(page: Page, { username, password }: Account) {
	await page.getByLabel('Username').fill(username);
	await page.getByLabel('Password').fill(password);
	await page.getByRole('button', { name: 'Sign in' }).click();
}

/**
 * Every test runs against corral-server, signed in as `role` when a test sets it, and fails on
 * any Content-Security-Policy violation.
 */
export const test = base.extend<{ role: Role | null }>({
	role: [null, { option: true }],
	baseURL: async ({}, use) => use(process.env.CORRAL_E2E_URL),
	storageState: async ({ role }, use) => use(role ? session(role) : undefined),
	page: async ({ page }, use) => {
		const violations: string[] = [];
		page.on('console', (message) => {
			if (message.text().includes('Content Security Policy')) {
				violations.push(message.text());
			}
		});
		await use(page);
		expect(violations).toEqual([]);
	}
});

export { expect };
