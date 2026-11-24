<script lang="ts">
	import type { Sandbox } from '#lib/api/generated/index.js';
	import { states, type State } from '#lib/sandbox-status.js';

	/** Tells screen readers when one of `sandboxes` changes state, as polling finds it. */
	let { sandboxes }: { sandboxes: Sandbox[] | undefined } = $props();

	let message = $state('');
	const seen = new Map<string, State>();

	$effect(() => {
		const changed: string[] = [];
		for (const sandbox of sandboxes ?? []) {
			const before = seen.get(sandbox.name);
			if (before !== undefined && before !== sandbox.state) {
				changed.push(`${sandbox.name} is ${states[sandbox.state].label.toLowerCase()}`);
			}
			seen.set(sandbox.name, sandbox.state);
		}
		if (changed.length > 0) {
			message = `${changed.join(', ')}.`;
		}
	});
</script>

<p class="sr-only" aria-live="polite">{message}</p>
