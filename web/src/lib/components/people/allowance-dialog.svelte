<script lang="ts">
	import { untrack } from 'svelte';
	import * as admin from '#lib/api/admin.js';
	import type { User } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';

	let {
		open = $bindable(),
		user,
		limit,
		onsaved
	}: {
		open: boolean;
		user: User;
		/** How many sandboxes someone without an allowance may have. */
		limit: number;
		onsaved: (user: User) => void;
	} = $props();

	let choice = $state<'limit' | 'own'>('limit');
	let count = $state<number | null>(null);

	$effect(() => {
		if (open) {
			untrack(() => {
				choice = user.allowance ? 'own' : 'limit';
				count = user.allowance?.maxSandboxes ?? null;
			});
		}
	});

	async function save() {
		onsaved(await admin.setAllowance(user, choice === 'own' ? count : null));
	}
</script>

<FormDialog
	bind:open
	title="Sandbox allowance for {user.handle}"
	description="How many sandboxes {user.handle} may have. The sandboxes they have now stay, even above it."
	submit="Save allowance"
	onsubmit={save}
>
	<Field.Set>
		<Field.Legend variant="label">Allowance</Field.Legend>
		<RadioGroup.Root bind:value={choice}>
			<Field.Field orientation="horizontal">
				<RadioGroup.Item
					value="limit"
					id="allowance-limit"
					aria-describedby="allowance-limit-hint"
				/>
				<Field.Content>
					<Field.Label for="allowance-limit">The usual limit</Field.Label>
					<Field.Description id="allowance-limit-hint">
						Up to {limit}, like everyone without an allowance.
					</Field.Description>
				</Field.Content>
			</Field.Field>
			<Field.Field orientation="horizontal">
				<RadioGroup.Item value="own" id="allowance-own" aria-describedby="allowance-own-hint" />
				<Field.Content>
					<Field.Label for="allowance-own">Their own</Field.Label>
					<Field.Description id="allowance-own-hint">
						Up to the number below, above or below the usual limit.
					</Field.Description>
				</Field.Content>
			</Field.Field>
		</RadioGroup.Root>
	</Field.Set>
	<Field.Field>
		<Field.Label for="allowance-count">Sandboxes</Field.Label>
		<Input
			id="allowance-count"
			type="number"
			inputmode="numeric"
			min={1}
			step={1}
			required={choice === 'own'}
			aria-describedby="allowance-count-hint"
			bind:value={count}
			oninput={() => (choice = 'own')}
		/>
		<Field.Description id="allowance-count-hint">A whole number, at least 1.</Field.Description>
	</Field.Field>
</FormDialog>
