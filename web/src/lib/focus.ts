/** Moves focus to the page's heading, as after deleting the thing that held focus. */
export function focusHeading() {
	const heading = document.querySelector<HTMLElement>('main h1');
	if (heading) {
		heading.tabIndex = -1;
		heading.focus({ preventScroll: true });
	}
}
