<script lang="ts">
	import type { Snippet } from 'svelte';
	import UnsavedGuard from '#lib/components/unsaved-guard.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import type { SettingsForm } from '#lib/settings-form.svelte.js';

	let {
		form,
		what,
		children
	}: {
		form: Pick<SettingsForm<unknown>, 'dirty' | 'saving' | 'failure' | 'save' | 'reset'>;
		/** What the page changes, as the question on leaving it unsaved names it. */
		what: string;
		children: Snippet;
	} = $props();

	function submit(event: SubmitEvent) {
		event.preventDefault();
		void form.save();
	}
</script>

<UnsavedGuard dirty={form.dirty} {what} />

<form class="grid gap-6" onsubmit={submit}>
	{@render children()}
	<div class="flex flex-wrap items-center gap-3">
		<Button type="submit" disabled={form.saving || !form.dirty}>
			{#if form.saving}
				<Spinner />
			{/if}
			Save
		</Button>
		{#if form.dirty}
			<Button type="button" variant="outline" disabled={form.saving} onclick={() => form.reset()}>
				Reset
			</Button>
		{/if}
		{#if form.failure}
			<p class="text-sm text-destructive first-letter:uppercase" role="alert">{form.failure}</p>
		{:else}
			<p class="text-sm text-muted-foreground" role="status">
				{form.dirty ? 'Unsaved changes' : ''}
			</p>
		{/if}
	</div>
</form>
