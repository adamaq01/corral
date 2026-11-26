<script lang="ts">
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import LockIcon from '@lucide/svelte/icons/lock';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import type { Row } from '#lib/settings.js';

	/** How the config file marks a row or a value: locked, shadowed, or not at all. */
	let { state, id }: { state: Row<unknown>['state']; id?: string } = $props();

	const marks = {
		locked: { icon: LockIcon, label: 'Set in config' },
		shadowed: { icon: EyeOffIcon, label: 'Shadowed by config' },
		editable: null
	};
	const mark = $derived(marks[state]);
</script>

{#if mark}
	<Badge variant="outline" {id}>
		<mark.icon aria-hidden="true" />
		{mark.label}
	</Badge>
{/if}
