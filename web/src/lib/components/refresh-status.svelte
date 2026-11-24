<script lang="ts" module>
	const clock = new Intl.DateTimeFormat(undefined, { timeStyle: 'medium' });
</script>

<script lang="ts">
	import CloudOffIcon from '@lucide/svelte/icons/cloud-off';
	import type { Remote } from '#lib/remote.svelte.js';

	/** With `time`, it also says when the value shown arrived. */
	let { remote, time = false }: { remote: Remote<unknown>; time?: boolean } = $props();
</script>

<p class="flex items-start gap-1.5 text-xs text-muted-foreground empty:hidden" role="status">
	{#if remote.refreshFailure}
		<CloudOffIcon class="mt-px size-3.5 shrink-0" aria-hidden="true" />
		<span>Could not refresh: {remote.refreshFailure}. This is the last state Corral reported.</span>
	{:else if time && remote.fetchedAt}
		<span>
			Updated <time datetime={remote.fetchedAt.toISOString()}>
				{clock.format(remote.fetchedAt)}
			</time>
		</span>
	{/if}
</p>
