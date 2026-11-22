<script lang="ts" module>
	import type { BadgeVariant } from '#lib/components/ui/badge/index.js';
	import type { SandboxStatus } from '#lib/api/generated/index.js';

	const states: Record<SandboxStatus['state'], { label: string; variant: BadgeVariant }> = {
		starting: { label: 'Starting', variant: 'outline' },
		running: { label: 'Running', variant: 'default' },
		sleeping: { label: 'Sleeping', variant: 'secondary' },
		failed: { label: 'Failed', variant: 'destructive' },
		deleting: { label: 'Deleting', variant: 'outline' }
	};
</script>

<script lang="ts">
	import { Badge } from '#lib/components/ui/badge/index.js';

	let { status }: { status: SandboxStatus } = $props();

	const shown = $derived(states[status.state]);
</script>

<div class="grid justify-items-start gap-1">
	<Badge variant={shown.variant}>{shown.label}</Badge>
	{#if status.state === 'failed'}
		<span class="max-w-32 truncate text-xs text-muted-foreground sm:max-w-48" title={status.detail}>
			{status.detail}
		</span>
	{/if}
</div>
