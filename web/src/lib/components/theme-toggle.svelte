<script lang="ts">
	import MoonIcon from '@lucide/svelte/icons/moon';
	import SunIcon from '@lucide/svelte/icons/sun';
	import { setMode, userPrefersMode } from 'mode-watcher';
	import { buttonVariants } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';

	const modes = [
		{ value: 'light', label: 'Light' },
		{ value: 'dark', label: 'Dark' },
		{ value: 'system', label: 'System' }
	] as const;

	type Mode = (typeof modes)[number]['value'];
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger
		class={buttonVariants({ variant: 'ghost', size: 'icon' })}
		aria-label="Theme"
		title="Theme"
	>
		<SunIcon class="scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" />
		<MoonIcon class="absolute scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0" />
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="end">
		<DropdownMenu.RadioGroup
			bind:value={() => userPrefersMode.current, (mode) => setMode(mode as Mode)}
		>
			{#each modes as mode (mode.value)}
				<DropdownMenu.RadioItem value={mode.value}>{mode.label}</DropdownMenu.RadioItem>
			{/each}
		</DropdownMenu.RadioGroup>
	</DropdownMenu.Content>
</DropdownMenu.Root>
