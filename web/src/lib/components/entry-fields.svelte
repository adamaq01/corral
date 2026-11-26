<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { EntryKind, StoredValue } from '#lib/api/generated/index.js';
	import ValueField from '#lib/components/value-field.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import type { EntryForm } from '#lib/entry-form.svelte.js';
	import { noun } from '#lib/setup.js';

	let {
		form,
		entry,
		current,
		taken = null,
		newSandboxes,
		children
	}: {
		form: EntryForm;
		/** The entry changed, or `null` for a new one. */
		entry: { kind: EntryKind; secret: boolean } | null;
		/** The changed entry's value, for a dialog that changes it too. */
		current?: StoredValue;
		/** Why another entry keeps the target from being used. */
		taken?: string | null;
		/** The label and hint of the switch that gives new sandboxes the entry, `null` for none. */
		newSandboxes: { label: string; hint: string } | null;
		/** The dialog's own fields, after the entry's flags. */
		children?: Snippet;
	} = $props();

	const problem = $derived(form.problem ?? taken);
	const reshaped = $derived(
		entry !== null && (entry.kind !== form.kind || entry.secret !== form.secret)
	);
</script>

{#snippet toggle(
	key: 'secret' | 'template' | 'executable' | 'applyToNew',
	label: string,
	hint: string
)}
	<Field.Field orientation="horizontal">
		<Switch id="entry-{key}" bind:checked={form[key]} aria-describedby="entry-{key}-hint" />
		<Field.Content>
			<Field.Label for="entry-{key}">{label}</Field.Label>
			<Field.Description id="entry-{key}-hint">{hint}</Field.Description>
		</Field.Content>
	</Field.Field>
{/snippet}

<Field.Set>
	<Field.Legend variant="label">Kind</Field.Legend>
	<RadioGroup.Root bind:value={form.kind} class="grid grid-cols-2 gap-2">
		{#each [{ kind: 'variable', label: 'Variable' }, { kind: 'file', label: 'File' }] as choice (choice.kind)}
			<Field.Field orientation="horizontal">
				<RadioGroup.Item value={choice.kind} id="entry-kind-{choice.kind}" />
				<Field.Label for="entry-kind-{choice.kind}">{choice.label}</Field.Label>
			</Field.Field>
		{/each}
	</RadioGroup.Root>
</Field.Set>

{#if form.variable}
	<Field.Field data-invalid={problem ? true : undefined}>
		<Field.Label for="entry-target">Name</Field.Label>
		<Input
			id="entry-target"
			aria-describedby="entry-target-hint"
			aria-invalid={problem ? true : undefined}
			bind:value={form.typed}
			required
			maxlength={64}
			autocomplete="off"
			autocapitalize="off"
			spellcheck={false}
			class="font-mono"
			placeholder="OPENAI_API_KEY"
		/>
		{#if problem}
			<Field.Error id="entry-target-hint">{problem}</Field.Error>
		{:else}
			<Field.Description id="entry-target-hint">
				Letters, digits and underscores, not starting with a digit. Case counts. PATH, HOME and the
				other names the sandbox sets, and names starting with CORRAL_, are reserved.
			</Field.Description>
		{/if}
	</Field.Field>
{:else}
	<Field.Field data-invalid={problem ? true : undefined}>
		<Field.Label for="entry-target">Path in the home</Field.Label>
		<Input
			id="entry-target"
			aria-describedby="entry-target-hint"
			aria-invalid={problem ? true : undefined}
			bind:value={form.typed}
			required
			maxlength={1040}
			autocomplete="off"
			autocapitalize="off"
			spellcheck={false}
			class="font-mono"
			placeholder="~/.config/tool/config.toml"
		/>
		{#if problem}
			<Field.Error id="entry-target-hint">{problem}</Field.Error>
		{:else}
			<Field.Description id="entry-target-hint">
				Inside /home/sandbox, as <code class="font-mono">~/.config/tool/config.toml</code>
				or <code class="font-mono">.config/tool/config.toml</code>. Missing directories are made.
			</Field.Description>
		{/if}
	</Field.Field>
{/if}

{#if entry === null || current}
	<ValueField
		id="entry-value"
		kind={form.kind}
		secret={form.secret}
		name={form.target}
		current={current ?? null}
		bind:value={form.value}
	/>
{/if}

<Field.Group class="gap-4">
	{@render toggle(
		'secret',
		'Secret',
		entry
			? 'Never shown again. Changing it moves the value and its overrides to or from the secret store.'
			: `Kept in the secret store, never shown again${form.variable ? '' : ', readable by the sandbox user alone'}.`
	)}
	{#if !form.variable}
		{@render toggle(
			'template',
			'Template',
			'Each {{ NAME }}, an upper-case variable name, takes that variable’s value in the sandbox. Any other {{ stays as written.'
		)}
		{@render toggle(
			'executable',
			'Executable',
			form.secret ? 'Written with mode 0700.' : 'Written with mode 0755, for a script.'
		)}
	{/if}
	{@render children?.()}
	{#if newSandboxes}
		{@render toggle('applyToNew', newSandboxes.label, newSandboxes.hint)}
	{/if}
</Field.Group>

{#if reshaped}
	<p class="text-sm text-muted-foreground" role="status">
		Corral checks that its value, and each sandbox's override, suits a {form.secret
			? 'secret'
			: 'plain'}
		{noun(form.kind)} before it saves.
	</p>
{/if}
