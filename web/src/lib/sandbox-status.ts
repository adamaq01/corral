import type { SandboxStatus } from '#lib/api/generated/index.js';

export type State = SandboxStatus['state'];

export type Power = 'wake' | 'stop' | 'restart';

type Presentation = {
	label: string;
	/** Still on its way to another state, so its status is worth fetching again soon. */
	settling: boolean;
	/** What its owner can do to its power from here. */
	powers: Power[];
	/** Whether it can be made the default, or deleted. */
	manageable: boolean;
};

export const states: Record<State, Presentation> = {
	starting: { label: 'Starting', settling: true, powers: ['stop'], manageable: true },
	running: { label: 'Running', settling: false, powers: ['stop', 'restart'], manageable: true },
	sleeping: { label: 'Sleeping', settling: false, powers: ['wake'], manageable: true },
	failed: { label: 'Failed', settling: false, powers: ['wake', 'restart'], manageable: true },
	deleting: { label: 'Deleting', settling: true, powers: [], manageable: false }
};
