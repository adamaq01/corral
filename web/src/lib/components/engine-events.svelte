<script lang="ts" module>
	const moment = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium' });
</script>

<script lang="ts">
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import type { EngineEvent } from '#lib/api/generated/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';

	let { events }: { events: EngineEvent[] } = $props();
</script>

{#if events.length === 0}
	<p class="text-muted-foreground">
		No recent events. The engine keeps them for a limited time only.
	</p>
{:else}
	<ol class="grid gap-1" aria-label="Recent events">
		{#each events as event, index (index)}
			{@const warning = event.kind === 'warning'}
			<li
				class={[
					'grid gap-1 rounded-md px-3 py-2',
					warning && 'border-l-4 border-destructive bg-muted/50 pl-2'
				]}
			>
				<div class="flex flex-wrap items-center gap-2">
					{#if warning}
						<Badge variant="destructive"><TriangleAlertIcon aria-hidden="true" />Warning</Badge>
					{/if}
					<span class="font-medium wrap-anywhere">{event.reason}</span>
				</div>
				{#if event.message}
					<p class="wrap-anywhere">{event.message}</p>
				{/if}
				<p class="text-xs text-muted-foreground">
					<span class="font-mono wrap-anywhere">{event.object}</span>
					{#if event.at}
						<span aria-hidden="true">·</span>
						<time class="whitespace-nowrap" datetime={event.at}>
							{moment.format(new Date(event.at))}
						</time>
					{/if}
				</p>
			</li>
		{/each}
	</ol>
{/if}
