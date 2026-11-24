<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import { toast } from 'svelte-sonner';
	import { copyText } from '#lib/clipboard.js';
	import { Button } from '#lib/components/ui/button/index.js';

	/** An icon button that copies `text`, which `label` names, as in "the SSH command for main". */
	let { text, label, class: className }: { text: string; label: string; class?: string } = $props();

	let copied = $state(false);
	let reset: ReturnType<typeof setTimeout> | undefined;

	async function copy() {
		try {
			await copyText(text);
		} catch {
			toast.error(`Copying failed. Copy it by hand: ${text}`);
			return;
		}
		copied = true;
		toast.success(`Copied ${label}`);
		clearTimeout(reset);
		reset = setTimeout(() => (copied = false), 2000);
	}
</script>

<Button
	variant="ghost"
	size="icon-sm"
	class={className}
	aria-label="Copy {label}"
	title="Copy {label}"
	onclick={copy}
>
	{#if copied}
		<CheckIcon />
	{:else}
		<CopyIcon />
	{/if}
</Button>
