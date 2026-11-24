<script lang="ts">
	import type { Snippet } from 'svelte';
	import { failureMessage } from '#lib/api/client.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { focusHeading } from '#lib/focus.js';

	let {
		open = $bindable(false),
		title,
		action,
		confirmName,
		onconfirm,
		children
	}: {
		open: boolean;
		title: string;
		/** The destructive button's label. */
		action: string;
		/** A name to type before the action can run, for an action that loses data for good. */
		confirmName?: string;
		/** Runs the action; the dialog closes when it succeeds and shows the error when it fails. */
		onconfirm: () => Promise<void>;
		/** What the action does, in a sentence or two. */
		children: Snippet;
	} = $props();

	let pending = $state(false);
	let error = $state<string | null>(null);
	let typed = $state('');
	let done = false;

	const ready = $derived(confirmName === undefined || typed === confirmName);

	async function confirm(event: SubmitEvent) {
		event.preventDefault();
		if (!ready) {
			return;
		}
		pending = true;
		error = null;
		done = false;
		try {
			await onconfirm();
			done = true;
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
		if (next) {
			done = false;
		} else {
			error = null;
			typed = '';
		}
	}}
>
	<AlertDialog.Content
		onCloseAutoFocus={(event) => {
			// What opened the dialog is usually gone with what it deleted.
			if (done) {
				event.preventDefault();
				focusHeading();
			}
		}}
	>
		<form class="grid min-w-0 gap-4" onsubmit={confirm}>
			<AlertDialog.Header>
				<AlertDialog.Title class="wrap-anywhere">{title}</AlertDialog.Title>
				<AlertDialog.Description class="grid gap-2">{@render children()}</AlertDialog.Description>
			</AlertDialog.Header>
			{#if confirmName !== undefined}
				<div class="grid gap-2">
					<Label for="confirm-name" class="block leading-normal">
						Type <span class="font-mono wrap-anywhere">{confirmName}</span> to confirm
					</Label>
					<Input
						id="confirm-name"
						bind:value={typed}
						autocomplete="off"
						autocapitalize="none"
						spellcheck={false}
						disabled={pending}
					/>
				</div>
			{/if}
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
