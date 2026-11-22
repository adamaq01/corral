<script lang="ts">
	import type { Snippet } from 'svelte';
	import { failureMessage } from '#lib/api/client.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';

	let {
		open = $bindable(false),
		title,
		description,
		submit,
		ready = true,
		onsubmit,
		children
	}: {
		open: boolean;
		title: string;
		description: string;
		/** The submit button's label. */
		submit: string;
		/** Whether the form can be sent yet. */
		ready?: boolean;
		/** Sends the form; the dialog closes when it succeeds and shows the error when it fails. */
		onsubmit: () => Promise<void>;
		/** The form's fields. */
		children: Snippet;
	} = $props();

	let pending = $state(false);
	let error = $state<string | null>(null);

	async function send(event: SubmitEvent) {
		event.preventDefault();
		pending = true;
		error = null;
		try {
			await onsubmit();
			open = false;
		} catch (failure) {
			error = failureMessage(failure);
		} finally {
			pending = false;
		}
	}
</script>

<Dialog.Root
	bind:open={() => open, (next) => (open = next || pending)}
	onOpenChangeComplete={(next) => {
		if (!next) {
			error = null;
		}
	}}
>
	<Dialog.Content>
		<form class="grid gap-4" onsubmit={send}>
			<Dialog.Header>
				<Dialog.Title>{title}</Dialog.Title>
				<Dialog.Description>{description}</Dialog.Description>
			</Dialog.Header>
			{@render children()}
			{#if error}
				<p class="text-sm text-destructive first-letter:uppercase" role="alert">{error}</p>
			{/if}
			<Dialog.Footer>
				<Dialog.Close disabled={pending}>
					{#snippet child({ props })}
						<Button variant="outline" {...props}>Cancel</Button>
					{/snippet}
				</Dialog.Close>
				<Button type="submit" disabled={pending || !ready}>
					{#if pending}
						<Spinner />
					{/if}
					{submit}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
