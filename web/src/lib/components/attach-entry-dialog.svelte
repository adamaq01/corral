<script lang="ts">
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import SearchIcon from '@lucide/svelte/icons/search';
	import VariableIcon from '@lucide/svelte/icons/variable';
	import { SvelteSet } from 'svelte/reactivity';
	import { toast } from 'svelte-sonner';
	import { failureMessage } from '#lib/api/client.js';
	import { setup } from '#lib/api/setup.js';
	import type { SetupEntry } from '#lib/api/generated/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Spinner } from '#lib/components/ui/spinner/index.js';
	import { summary, where } from '#lib/setup.js';

	let {
		open = $bindable(),
		sandbox,
		entries,
		onAttached
	}: {
		open: boolean;
		sandbox: string;
		/** The entries not attached to the sandbox yet. */
		entries: SetupEntry[];
		onAttached: () => Promise<void>;
	} = $props();

	const chosen = new SvelteSet<string>();
	let search = $state('');
	let pending = $state(false);
	let errors = $state<Record<string, string>>({});

	const shown = $derived(
		entries.filter((entry) => where(entry).toLowerCase().includes(search.trim().toLowerCase()))
	);

	function toggle(entry: SetupEntry, checked: boolean) {
		if (checked) {
			chosen.add(entry.id);
		} else {
			chosen.delete(entry.id);
		}
	}

	// Variables first, so a template finds the variables it refers to already attached.
	function order(a: SetupEntry, b: SetupEntry) {
		return (
			Number(a.template) - Number(b.template) ||
			(a.kind === b.kind ? 0 : a.kind === 'variable' ? -1 : 1)
		);
	}

	async function attach(event: SubmitEvent) {
		event.preventDefault();
		pending = true;
		errors = {};
		const picked = entries.filter((entry) => chosen.has(entry.id)).sort(order);
		let attached = 0;
		for (const entry of picked) {
			try {
				await setup.attach(sandbox, entry.id);
				chosen.delete(entry.id);
				attached += 1;
			} catch (failure) {
				errors[entry.id] = failureMessage(failure);
			}
		}
		pending = false;
		if (attached > 0) {
			toast.success(`Attached ${attached} to ${sandbox}`);
			await onAttached();
		}
		if (Object.keys(errors).length === 0) {
			open = false;
		}
	}
</script>

<Dialog.Root
	bind:open={() => open, (next) => (open = next || pending)}
	onOpenChangeComplete={(next) => {
		if (!next) {
			errors = {};
			search = '';
			chosen.clear();
		}
	}}
>
	<Dialog.Content class="max-h-[90dvh] overflow-y-auto">
		<form class="grid min-w-0 gap-4" onsubmit={attach}>
			<Dialog.Header>
				<Dialog.Title>Attach to {sandbox}</Dialog.Title>
				<Dialog.Description>
					Each one reaches the sandbox at its next start. Add new variables and files on
					<a class="underline underline-offset-4" href="/setup">Setup</a>.
				</Dialog.Description>
			</Dialog.Header>
			{#if entries.length === 0}
				<p class="text-sm text-muted-foreground">Every variable and file you have is attached.</p>
			{:else}
				{#if entries.length > 6}
					<div class="relative">
						<SearchIcon
							class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
							aria-hidden="true"
						/>
						<Input
							type="search"
							placeholder="Find by name or path"
							aria-label="Find by name or path"
							class="pl-8"
							bind:value={search}
						/>
					</div>
				{/if}
				<ul class="grid gap-1" aria-label="Variables and files to attach">
					{#each shown as entry (entry.id)}
						{@const Icon = entry.kind === 'variable' ? VariableIcon : FileTextIcon}
						<li>
							<label
								class="flex cursor-pointer items-start gap-3 rounded-md border px-3 py-2.5 has-data-checked:border-primary/50 has-data-checked:bg-primary/5"
							>
								<Checkbox
									class="mt-0.5"
									checked={chosen.has(entry.id)}
									disabled={pending}
									onCheckedChange={(checked) => toggle(entry, checked)}
								/>
								<span class="grid min-w-0 flex-1 gap-0.5">
									<span class="flex flex-wrap items-center gap-1.5 text-sm font-medium">
										<Icon class="size-3.5 text-muted-foreground" aria-hidden="true" />
										<span class="font-mono break-all">{where(entry)}</span>
										{#if entry.secret}
											<Badge variant="secondary">Secret</Badge>
										{/if}
										{#if entry.template}
											<Badge variant="outline">Template</Badge>
										{/if}
										{#if entry.applyToNew}
											<Badge variant="outline">New sandboxes</Badge>
										{/if}
									</span>
									<span class="truncate text-xs text-muted-foreground">
										{entry.kind === 'variable' || entry.value.shown !== 'text'
											? summary(entry.value)
											: entry.template && entry.references.length > 0
												? `Refers to ${entry.references.join(', ')}`
												: 'Text'}
									</span>
									{#if errors[entry.id]}
										<span class="text-sm text-destructive first-letter:uppercase" role="alert">
											{errors[entry.id]}
										</span>
									{/if}
								</span>
							</label>
						</li>
					{:else}
						<li class="text-sm text-muted-foreground">Nothing matches.</li>
					{/each}
				</ul>
			{/if}
			<Dialog.Footer>
				<Dialog.Close disabled={pending}>
					{#snippet child({ props })}
						<Button variant="outline" {...props}>Cancel</Button>
					{/snippet}
				</Dialog.Close>
				<Button type="submit" disabled={pending || chosen.size === 0}>
					{#if pending}
						<Spinner />
					{/if}
					{chosen.size > 1 ? `Attach ${chosen.size}` : 'Attach'}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
