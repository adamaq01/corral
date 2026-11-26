<script lang="ts">
	import LockIcon from '@lucide/svelte/icons/lock';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';

	let {
		id,
		label,
		value = $bindable(),
		locked = false,
		error = null,
		oncopy = null
	}: {
		id: string;
		label: string;
		value: string;
		locked?: boolean;
		error?: string | null;
		/** Copies the address into the other providers' fields, offered under it when given. */
		oncopy?: (() => void) | null;
	} = $props();
</script>

<div class="grid gap-2">
	<Field.Field data-invalid={error ? true : undefined}>
		<div class="flex flex-wrap items-center gap-2">
			<Field.Label for={id}>{label}</Field.Label>
			{#if locked}
				<Badge variant="outline" id="{id}-locked">
					<LockIcon aria-hidden="true" />
					Set by the admin
				</Badge>
			{/if}
		</div>
		<Input
			{id}
			type="url"
			bind:value
			placeholder="https://llm.home.example"
			disabled={locked}
			aria-invalid={error ? true : undefined}
			aria-describedby={locked ? `${id}-locked` : `${id}-hint`}
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
		/>
		{#if error}
			<Field.Error id="{id}-hint" class="first-letter:uppercase">{error}</Field.Error>
		{:else if !locked}
			<Field.Description id="{id}-hint">
				Optional. Used exactly as typed, for a gateway or proxy.
			</Field.Description>
		{/if}
	</Field.Field>
	{#if oncopy}
		<Button variant="link" size="sm" class="h-auto w-fit p-0" onclick={oncopy}>
			Use this address for every provider
		</Button>
	{/if}
</div>
