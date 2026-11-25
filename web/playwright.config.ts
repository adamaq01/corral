import { defineConfig, devices } from '@playwright/test';

// Docker adds and removes a network interface on the host as a sandbox starts and stops, and
// Chromium then fails loads in flight with ERR_NETWORK_CHANGED. Tests that start sandboxes run
// after the others, one at a time.
const startsSandboxes = /(admin|connect|delete-from-inside|journey|lifecycle|sleep)\.spec\.ts$/;
// The screenshot specs check each page in four views (e2e/views.ts). chromium checks two, light at
// desktop width and dark at phone width; other-views, which the nightly run adds, the other two.
const otherViews = /light, phone|dark, desktop/;

export default defineConfig({
	testDir: 'e2e',
	globalSetup: './e2e/corral.ts',
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
	use: { trace: 'retain-on-failure' },
	projects: [
		{
			name: 'chromium',
			use: { ...devices['Desktop Chrome'] },
			testIgnore: startsSandboxes,
			grepInvert: otherViews
		},
		{
			name: 'sandboxes',
			use: { ...devices['Desktop Chrome'] },
			testMatch: startsSandboxes,
			dependencies: ['chromium'],
			workers: 1
		},
		{
			name: 'other-views',
			use: { ...devices['Desktop Chrome'] },
			testMatch: /screenshots\.spec\.ts$/,
			grep: otherViews
		}
	]
});
