<script lang="ts">
	import { untrack } from 'svelte';
	import { accounts } from '#lib/api/accounts.js';
	import { MIN_PASSWORD_CHARS, type LocalAccount } from '#lib/api/generated/index.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import PasswordInput from '#lib/components/password-input.svelte';
	import * as Field from '#lib/components/ui/field/index.js';

	let {
		open = $bindable(),
		url,
		account,
		onreset
	}: {
		open: boolean;
		/** The `accounts` URL of the provider that keeps them. */
		url: string;
		account: LocalAccount;
		onreset: () => void;
	} = $props();

	let password = $state('');

	$effect(() => {
		if (open) {
			untrack(() => (password = ''));
		}
	});

	async function reset() {
		await accounts(url).setPassword(account.username, password);
		onreset();
	}
</script>

<FormDialog
	bind:open
	title="Reset the password of {account.username}"
	description="They sign in with the new password from now on."
	submit="Reset password"
	onsubmit={reset}
>
	<Field.Field>
		<Field.Label for="reset-password">New password</Field.Label>
		<PasswordInput
			id="reset-password"
			autocomplete="new-password"
			minlength={MIN_PASSWORD_CHARS}
			required
			aria-describedby="reset-password-hint"
			bind:value={password}
		/>
		<Field.Description id="reset-password-hint">
			At least {MIN_PASSWORD_CHARS} characters.
		</Field.Description>
	</Field.Field>
</FormDialog>
