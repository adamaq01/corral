import { join } from 'node:path';
import AxeBuilder from '@axe-core/playwright';
import type { Page } from '@playwright/test';
import { expect, test } from './fixtures';

const directory = process.env.CORRAL_SCREENSHOTS;

const widths = { desktop: 1280, phone: 390 };

/**
 * Defines `body` once for each theme at desktop and phone width, as `<scheme>, <device>`, with
 * `look` checking each view it reaches.
 */
export function inEveryView(
	body: (
		look: (page: Page, name: string, options?: { fullPage?: boolean }) => Promise<void>
	) => void
) {
	for (const scheme of ['light', 'dark'] as const) {
		for (const [device, width] of Object.entries(widths)) {
			test.describe(`${scheme}, ${device}`, () => {
				test.use({
					colorScheme: scheme,
					viewport: { width, height: device === 'phone' ? 844 : 800 }
				});
				body(async (page, name, { fullPage = true } = {}) => {
					await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
					// A dialog fading in would show axe colours half way to their own.
					await page.waitForFunction(() =>
						document
							.getAnimations()
							.every(
								(animation) =>
									animation.playState !== 'running' ||
									animation.effect?.getTiming().iterations === Infinity
							)
					);
					if (directory) {
						await page.screenshot({
							path: join(directory, `${name}-${scheme}-${width}.png`),
							animations: 'disabled',
							fullPage
						});
					}
					await expectAccessible(page);
				});
			});
		}
	}
}

/**
 * The view has no serious or critical axe violation, headings that start at one h1 and skip no
 * level, no sideways scrolling, and a visible focus indicator on every element Tab reaches.
 */
async function expectAccessible(page: Page) {
	const { violations } = await new AxeBuilder({ page }).analyze();
	const severe = violations
		.filter((violation) => violation.impact === 'serious' || violation.impact === 'critical')
		.map(({ id, nodes }) => ({
			id,
			nodes: nodes.map(
				(node) => `${node.target.join(' ')}: ${node.html.slice(0, 120)} ${node.failureSummary}`
			)
		}));
	expect(severe).toEqual([]);

	const levels = await page
		.locator('h1, h2, h3, h4, h5, h6')
		.filter({ visible: true })
		.evaluateAll((headings) => headings.map((heading) => Number(heading.tagName[1])));
	expect(
		levels.filter((level) => level === 1),
		'one h1'
	).toHaveLength(1);
	levels.forEach((level, index) => {
		const previous = index === 0 ? 0 : levels[index - 1];
		expect(level - previous, `heading levels ${levels.join(', ')}`).toBeLessThanOrEqual(1);
	});

	expect(
		await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth),
		'sideways scroll'
	).toBeLessThanOrEqual(0);

	await expectVisibleFocus(page);
}

/** Tabs through the view and back to where focus started, checking each stop shows focus. */
async function expectVisibleFocus(page: Page) {
	await page.addStyleTag({ content: '*, *::before, *::after { transition: none !important; }' });
	await page.evaluate(() =>
		document
			.querySelectorAll('[data-tab-stop]')
			.forEach((stop) => stop.removeAttribute('data-tab-stop'))
	);
	for (let stop = 0; stop < 100; stop++) {
		await page.keyboard.press('Tab');
		const focus = await page.evaluate(() => {
			const element = document.activeElement;
			if (
				!(element instanceof HTMLElement) ||
				element === document.body ||
				element.dataset.tabStop
			) {
				return null;
			}
			element.dataset.tabStop = 'seen';
			const look = () => {
				const style = getComputedStyle(element);
				const outlined =
					style.outlineStyle !== 'none' &&
					style.outlineWidth !== '0px' &&
					!/^(transparent|rgba\(0, 0, 0, 0\))$|\/ 0\)$/.test(style.outlineColor);
				return [
					outlined ? `${style.outlineStyle} ${style.outlineWidth} ${style.outlineColor}` : 'none',
					style.boxShadow,
					style.borderColor,
					style.backgroundColor
				].join(' | ');
			};
			const focused = look();
			element.blur();
			const blurred = look();
			element.focus();
			const label = element.getAttribute('aria-label') ?? element.textContent ?? '';
			return {
				name: `${element.tagName.toLowerCase()} "${label.trim().slice(0, 40)}"`,
				visible: focused !== blurred
			};
		});
		if (!focus) {
			break;
		}
		expect(focus.visible, `focus shows on ${focus.name}`).toBe(true);
	}
}
