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
		ready = true,
		onconfirm,
		children,
		fields
	}: {
		open: boolean;
		title: string;
		/** The destructive button's label. */
		action: string;
		/** Whether the destructive button can be pressed yet. */
		ready?: boolean;
		/** Runs the action; the dialog closes when it succeeds and shows the error when it fails. */
		onconfirm: () => Promise<void>;
		/** What the action does, in a sentence or two. */
		children: Snippet;
		/** Anything to fill in before the action can run. */
		fields?: Snippet;
	} = $props();

	let pending = $state(false);
	let error = $state<string | null>(null);

	async function confirm(event: SubmitEvent) {
		event.preventDefault();
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
		<form class="grid gap-4" onsubmit={confirm}>
			<AlertDialog.Header>
				<AlertDialog.Title>{title}</AlertDialog.Title>
				<AlertDialog.Description class="grid gap-2">{@render children()}</AlertDialog.Description>
			</AlertDialog.Header>
			{@render fields?.()}
			{#if error}
				<p class="text-sm text-destructive first-letter:uppercase" role="alert">{error}</p>
			{/if}
			<AlertDialog.Footer>
				<AlertDialog.Cancel type="button" disabled={pending}>Cancel</AlertDialog.Cancel>
				<Button type="submit" variant="destructive" disabled={pending || !ready}>
					{#if pending}
						<Spinner />
					{/if}
					{action}
				</Button>
			</AlertDialog.Footer>
		</form>
	</AlertDialog.Content>
</AlertDialog.Root>
