import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
	testDir: 'e2e',
	globalSetup: './e2e/corral.ts',
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
	use: { trace: 'retain-on-failure' },
	projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }]
});
