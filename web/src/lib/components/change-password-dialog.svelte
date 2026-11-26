<script lang="ts">
	import { untrack } from 'svelte';
	import { changePassword } from '#lib/api/accounts.js';
	import { ApiError } from '#lib/api/client.js';
	import { MIN_PASSWORD_CHARS } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import PasswordInput from '#lib/components/password-input.svelte';
	import * as Field from '#lib/components/ui/field/index.js';

	let {
		open = $bindable(),
		onchanged
	}: {
		open: boolean;
		onchanged: () => void;
	} = $props();

	let current = $state('');
	let password = $state('');
	/** Whether Corral refused the current password as typed. */
	let wrong = $state(false);

	$effect(() => {
		if (open) {
			untrack(() => {
				current = '';
				password = '';
				wrong = false;
			});
		}
	});

	async function change(): Promise<void | false> {
		try {
			await changePassword(current, password);
		} catch (failure) {
			if (failure instanceof ApiError && failure.status === 403) {
				wrong = true;
				return false;
			}
			throw failure;
		}
		onchanged();
	}
</script>

<FormDialog
	bind:open
	title="Change your password"
	description="You sign in with the new password from now on."
	submit="Change password"
	onsubmit={change}
>
	<Field.Field data-invalid={wrong ? true : undefined}>
		<Field.Label for="current-password">Current password</Field.Label>
		<PasswordInput
			id="current-password"
			autocomplete="current-password"
			required
			aria-invalid={wrong ? true : undefined}
			aria-describedby={wrong ? 'current-password-error' : undefined}
			bind:value={current}
			oninput={() => (wrong = false)}
		/>
		{#if wrong}
			<Field.Error id="current-password-error">Wrong password.</Field.Error>
		{/if}
	</Field.Field>
	<Field.Field>
		<Field.Label for="new-password">New password</Field.Label>
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
</FormDialog>
