<script lang="ts">
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { failureMessage } from '#lib/api/client.js';
	import { environment } from '#lib/api/environment.js';
	import type { EnvironmentEntry } from '#lib/api/generated/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Item from '#lib/components/ui/item/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { summary, where } from '#lib/environment.js';

	let {
		open = $bindable(),
		sandbox,
		entries,
		onAttached
	}: {
		open: boolean;
		sandbox: string;
		/** The entries not attached to the sandbox yet. */
		entries: EnvironmentEntry[];
		onAttached: () => Promise<void>;
	} = $props();

	let pending = $state<string | null>(null);
	let error = $state<{ id: string; message: string } | null>(null);

	async function attach(entry: EnvironmentEntry) {
		pending = entry.id;
		error = null;
		try {
			await environment.attach(sandbox, entry.id);
			await onAttached();
			open = false;
		} catch (failure) {
			error = { id: entry.id, message: failureMessage(failure) };
		} finally {
			pending = null;
		}
	}
</script>

<Dialog.Root
	bind:open={() => open, (next) => (open = next || pending !== null)}
	onOpenChangeComplete={(next) => {
		if (!next) {
			error = null;
		}
	}}
>
	<Dialog.Content class="max-h-[90dvh] overflow-y-auto">
		<Dialog.Header>
			<Dialog.Title>Attach to {sandbox}</Dialog.Title>
			<Dialog.Description>
				It reaches the sandbox at its next start. Add new variables and files on
				<a class="underline underline-offset-4" href="/environment">Environment</a>.
			</Dialog.Description>
		</Dialog.Header>
		{#if entries.length === 0}
			<p class="text-sm text-muted-foreground">Every variable and file you have is attached.</p>
		{:else}
			<ul class="grid gap-2" aria-label="Variables and files to attach">
				{#each entries as entry (entry.id)}
					{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
					<li>
						<Item.Root variant="outline" size="sm">
							<Item.Media variant="icon">
								<Icon class="text-muted-foreground" />
							</Item.Media>
							<Item.Content class="min-w-0">
								<Item.Title class="max-w-full font-mono break-all">
									{where(entry)}
									{#if entry.secret}
										<Badge variant="secondary">Secret</Badge>
									{/if}
								</Item.Title>
								<Item.Description class="break-all">
									{entry.kind === 'variable' || entry.value.shown !== 'text'
										? summary(entry.value)
										: 'Text'}
								</Item.Description>
								{#if error?.id === entry.id}
									<p class="text-sm text-destructive first-letter:uppercase" role="alert">
										{error.message}
									</p>
								{/if}
							</Item.Content>
							<Item.Actions>
								<Button
									variant="outline"
									size="sm"
									disabled={pending !== null}
									onclick={() => attach(entry)}
								>
									{#if pending === entry.id}
										<Spinner role="presentation" aria-label={undefined} aria-hidden="true" />
									{/if}
									Attach<span class="sr-only"> {where(entry)}</span>
								</Button>
							</Item.Actions>
						</Item.Root>
					</li>
				{/each}
			</ul>
		{/if}
		<Dialog.Footer>
			<Dialog.Close disabled={pending !== null}>
				{#snippet child({ props })}
					<Button variant="outline" {...props}>Done</Button>
				{/snippet}
			</Dialog.Close>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
