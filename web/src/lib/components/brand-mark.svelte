<script lang="ts">
	import FenceIcon from '@lucide/svelte/icons/fence';
	import { cn } from '#lib/utils.js';

	let { class: className }: { class?: string } = $props();
</script>

<span
	class={cn(
		'flex aspect-square size-8 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground',
		className
	)}
	aria-hidden="true"
>
	<FenceIcon class="size-4" />
</span>
