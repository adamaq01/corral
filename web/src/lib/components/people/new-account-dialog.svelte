<script lang="ts">
	import { untrack } from 'svelte';
	import { accounts } from '#lib/api/accounts.js';
	import { MIN_PASSWORD_CHARS, type Role } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import PasswordInput from '#lib/components/password-input.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import * as RadioGroup from '#lib/components/ui/radio-group/index.js';

	let {
		open = $bindable(),
		url,
		oncreated
	}: {
		open: boolean;
		/** The `accounts` URL of the provider that keeps them. */
		url: string;
		oncreated: (username: string) => Promise<void>;
	} = $props();

	const roles: { value: Role; label: string; description: string }[] = [
		{
			value: 'member',
			label: 'Member',
			description: 'Manages their own sandboxes, SSH keys and environment.'
		},
		{
			value: 'admin',
			label: 'Admin',
			description: 'Also manages Settings and People, and sees every sandbox.'
		}
	];

	let username = $state('');
	let password = $state('');
	let role = $state<Role>('member');

	$effect(() => {
		if (open) {
			untrack(() => {
				username = '';
				password = '';
				role = 'member';
			});
		}
	});

	async function create() {
		await accounts(url).create({ username, password, role });
		await oncreated(username);
	}
</script>

<FormDialog
	bind:open
	title="New local account"
	description="The person signs in with this username and password."
	submit="Create account"
	onsubmit={create}
>
	<Field.Field>
		<Field.Label for="new-username">Username</Field.Label>
		<Input
			id="new-username"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			pattern={'[a-z0-9\\-]{1,32}'}
			required
			aria-describedby="new-username-hint"
			bind:value={username}
		/>
		<Field.Description id="new-username-hint">
			Up to 32 lowercase letters, digits and dashes. It becomes their handle.
		</Field.Description>
	</Field.Field>
	<Field.Field>
		<Field.Label for="new-password">Password</Field.Label>
		<PasswordInput
			id="new-password"
			autocomplete="new-password"
			minlength={MIN_PASSWORD_CHARS}
			required
			aria-describedby="new-password-hint"
			bind:value={password}
		/>
		<Field.Description id="new-password-hint">
			At least {MIN_PASSWORD_CHARS} characters.
		</Field.Description>
	</Field.Field>
	<Field.Set>
		<Field.Legend variant="label">Role</Field.Legend>
		<RadioGroup.Root bind:value={role}>
			{#each roles as choice (choice.value)}
				<Field.Field orientation="horizontal">
					<RadioGroup.Item
						value={choice.value}
						id="role-{choice.value}"
						aria-describedby="role-{choice.value}-hint"
					/>
					<Field.Content>
						<Field.Label for="role-{choice.value}">{choice.label}</Field.Label>
						<Field.Description id="role-{choice.value}-hint">
							{choice.description}
						</Field.Description>
					</Field.Content>
				</Field.Field>
			{/each}
		</RadioGroup.Root>
	</Field.Set>
</FormDialog>
