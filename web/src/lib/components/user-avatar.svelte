<script lang="ts">
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import { handleColour, initials } from '#lib/avatar.js';
	import type { Handle, PictureUrl } from '#lib/api/generated/index.js';

	let {
		handle,
		picture,
		size = 'default',
		class: className
	}: {
		handle: Handle;
		picture: PictureUrl | null;
		size?: 'default' | 'sm' | 'lg';
		class?: string;
	} = $props();
</script>

<Avatar.Root {size} class={className}>
	{#if picture}
		<Avatar.Image src={picture} alt="" />
	{/if}
	<Avatar.Fallback
		class="font-medium text-white"
		style="background-color: {handleColour(handle)}"
		aria-hidden="true"
	>
		{initials(handle)}
	</Avatar.Fallback>
</Avatar.Root>
