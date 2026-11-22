<script lang="ts">
	import type { Snippet } from 'svelte';
	import { failureMessage } from '#lib/api/client.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';

	let {
		open = $bindable(false),
		title,
		action,
		onconfirm,
		children
	}: {
		open: boolean;
		title: string;
		/** The destructive button's label. */
		action: string;
		/** Runs the action; the dialog closes when it succeeds and shows the error when it fails. */
		onconfirm: () => Promise<void>;
		/** What the action does, in a sentence or two. */
		children: Snippet;
	} = $props();

	let pending = $state(false);
	let error = $state<string | null>(null);

	async function confirm() {
		pending = true;
		error = null;
		try {
			await onconfirm();
			open = false;
		} catch (failure) {
			error = failureMessage(failure);
		} finally {
			pending = false;
		}
	}
</script>

<AlertDialog.Root
	bind:open={() => open, (next) => (open = next || pending)}
	onOpenChangeComplete={(next) => {
		if (!next) {
			error = null;
		}
	}}
>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{title}</AlertDialog.Title>
			<AlertDialog.Description>{@render children()}</AlertDialog.Description>
		</AlertDialog.Header>
		{#if error}
			<p class="text-sm text-destructive first-letter:uppercase" role="alert">{error}</p>
		{/if}
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={pending}>Cancel</AlertDialog.Cancel>
			<Button variant="destructive" disabled={pending} onclick={confirm}>
				{#if pending}
					<Spinner />
				{/if}
				{action}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
