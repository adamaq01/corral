<script lang="ts">
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import MoonIcon from '@lucide/svelte/icons/moon';
	import type { SandboxStatus } from '#lib/api/generated/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { states } from '#lib/sandbox-status.js';

	/** With `detail`, a failed sandbox's reason shows under the badge, wrapped to fit a table cell. */
	let { status, detail = false }: { status: SandboxStatus; detail?: boolean } = $props();
</script>

{#snippet badge()}
	<Badge
		variant={status.state === 'failed' ? 'destructive' : 'outline'}
		title={status.state === 'failed' ? status.detail : undefined}
	>
		{#if status.state === 'running'}
			<span class="size-2 rounded-full bg-emerald-500" aria-hidden="true"></span>
		{:else if status.state === 'sleeping'}
			<MoonIcon aria-hidden="true" />
		{:else if status.state === 'failed'}
			<CircleAlertIcon aria-hidden="true" />
		{:else}
			<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
		{/if}
		{states[status.state].label}
	</Badge>
{/snippet}

{#if detail}
	<div class="grid justify-items-start gap-1">
		{@render badge()}
		{#if status.state === 'failed'}
			<span
				class="max-w-32 text-xs wrap-break-word whitespace-normal text-muted-foreground first-letter:uppercase sm:max-w-56"
			>
				{status.detail}
			</span>
		{/if}
	</div>
{:else}
	{@render badge()}
{/if}
