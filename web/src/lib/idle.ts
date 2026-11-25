import type { Idle } from '#lib/api/generated/index.js';

const MINUTE = 60;
const HOUR = 60 * MINUTE;

/** `30 min`, `2 h`, `1 h 30 min`. */
export function duration(secs: number): string {
	const minutes = Math.round(secs / MINUTE);
	const [hours, rest] = [Math.floor(minutes / 60), minutes % 60];
	if (hours === 0) {
		return `${rest} min`;
	}
	return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

const TIMEOUTS = [15 * MINUTE, 30 * MINUTE, HOUR, 2 * HOUR, 4 * HOUR, 8 * HOUR, 24 * HOUR];

/** The timeouts an owner may pick, the default and the current one among them. */
export function timeoutChoices(idle: Idle): number[] {
	const allowed = TIMEOUTS.filter(
		(secs) => secs >= idle.minTimeoutSecs && secs <= idle.maxTimeoutSecs
	);
	return [...new Set([...allowed, idle.defaultTimeoutSecs, idle.timeoutSecs])]
		.filter((secs) => secs <= idle.maxTimeoutSecs)
		.sort((a, b) => a - b);
}

export type KeepAwakeChoice = { label: string; until: Date };

/** 1 h, 4 h and until 9:00 tomorrow, those the admin allows. */
export function keepAwakeChoices(idle: Idle, now: Date): KeepAwakeChoice[] {
	const tomorrow = new Date(now);
	tomorrow.setDate(tomorrow.getDate() + 1);
	tomorrow.setHours(9, 0, 0, 0);
	const furthest = now.getTime() + idle.maxKeepAwakeSecs * 1000;
	return [
		{ label: '1 hour', until: new Date(now.getTime() + HOUR * 1000) },
		{ label: '4 hours', until: new Date(now.getTime() + 4 * HOUR * 1000) },
		{ label: 'Until tomorrow 9:00', until: tomorrow }
	].filter((choice) => choice.until.getTime() <= furthest);
}
