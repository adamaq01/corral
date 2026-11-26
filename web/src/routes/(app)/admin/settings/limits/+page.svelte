<script lang="ts">
	import * as settings from '#lib/api/settings.js';
	import ListSkeleton from '#lib/components/list-skeleton.svelte';
	import LoadFailed from '#lib/components/load-failed.svelte';
	import ConfigBadge from '#lib/components/settings/config-badge.svelte';
	import SaveForm from '#lib/components/settings/save-form.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { SettingsForm } from '#lib/settings-form.svelte.js';
	import { limitsChange, limitsView, type Locked } from '#lib/settings.js';

	const form = new SettingsForm(
		async () => limitsView(await settings.limits()),
		(view) => settings.setLimits(limitsChange(view)),
		'Saved the limits'
	);
</script>

{#snippet limit(id: string, label: string, hint: string, field: Locked<number | null>)}
	<Field.Field>
		<div class="flex flex-wrap items-center gap-2">
			<Field.Label for={id}>{label}</Field.Label>
			{#if field.locked}
				<ConfigBadge state="locked" id="{id}-locked" />
			{/if}
		</div>
		<Input
			{id}
			type="number"
			inputmode="numeric"
			min={1}
			step={1}
			required
			disabled={field.locked}
			aria-describedby={field.locked ? `${id}-locked ${id}-hint` : `${id}-hint`}
			bind:value={field.value}
		/>
		<Field.Description id="{id}-hint">{hint}</Field.Description>
	</Field.Field>
{/snippet}

{#if form.loaded.current.state === 'failed'}
	<LoadFailed
		title="Loading the limits failed"
		error={form.loaded.current.error}
		onretry={() => form.loaded.reload()}
	/>
{:else if !form.edited}
	<ListSkeleton what="the limits" />
{:else}
	{@const view = form.edited}
	<SaveForm {form} what="the limits">
		<Card.Root>
			<Card.Header>
				<Card.Title><h2>Sandboxes</h2></Card.Title>
				<Card.Description>
					How many sandboxes members may have and run. A member's own allowance is on
					<a
						href="/admin/people"
						class="rounded-sm underline underline-offset-4 focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
					>
						People
					</a>.
				</Card.Description>
			</Card.Header>
			<Card.Content class="grid gap-6 sm:grid-cols-2">
				{@render limit(
					'max-sandboxes',
					'Sandboxes per member',
					'How many sandboxes each member may have. Lowering it leaves the sandboxes they have.',
					view.maxSandboxesPerMember
				)}
				{@render limit(
					'max-running',
					'Running at once',
					'How many sandboxes may run at the same time, across everyone.',
					view.maxRunning
				)}
			</Card.Content>
		</Card.Root>
	</SaveForm>
{/if}
