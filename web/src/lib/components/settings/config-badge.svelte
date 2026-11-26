<script lang="ts">
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import LockIcon from '@lucide/svelte/icons/lock';
	import PackageIcon from '@lucide/svelte/icons/package';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import type { Row } from '#lib/settings.js';

	/** How a row or a value is marked: set in config, shadowed by config, built in, or not at all. */
	let { state, id }: { state: Row<unknown>['state']; id?: string } = $props();

	const marks = {
		locked: { icon: LockIcon, label: 'Set in config' },
		shadowed: { icon: EyeOffIcon, label: 'Shadowed by config' },
		builtin: { icon: PackageIcon, label: 'Built in' },
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
