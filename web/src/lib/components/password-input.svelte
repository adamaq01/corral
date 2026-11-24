<script lang="ts">
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import type { HTMLInputAttributes } from 'svelte/elements';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';

	/** A password field with a button that shows what was typed, to catch a typo. */
	let {
		value = $bindable(''),
		...rest
	}: Omit<HTMLInputAttributes, 'type' | 'value' | 'files'> & { value?: string } = $props();

	let shown = $state(false);
</script>

<div class="relative">
	<Input {...rest} type={shown ? 'text' : 'password'} class="pr-10" bind:value />
	<Button
		type="button"
		variant="ghost"
		size="icon-sm"
		class="absolute top-1/2 right-1 -translate-y-1/2"
		aria-label="Show password"
		aria-pressed={shown}
		title={shown ? 'Hide password' : 'Show password'}
		onclick={() => (shown = !shown)}
	>
		{#if shown}
			<EyeOffIcon />
		{:else}
			<EyeIcon />
		{/if}
	</Button>
</div>
