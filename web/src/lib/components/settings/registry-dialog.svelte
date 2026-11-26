<script lang="ts">
	import { untrack } from 'svelte';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import type { RegistryEntry } from '#lib/settings.js';

	let {
		open = $bindable(),
		registry,
		refuse,
		onsave
	}: {
		open: boolean;
		/** The registry to change, or `null` to add one. */
		registry: RegistryEntry | null;
		/** Why a host cannot name a new registry, if it cannot. */
		refuse: (host: string) => string | null;
		onsave: (registry: RegistryEntry) => void;
	} = $props();

	let fields = $state({ registry: '', username: '', passwordEnv: '', plainHttp: false });

	$effect(() => {
		if (open) {
			untrack(() => {
				fields = {
					registry: registry?.registry ?? '',
					username: registry?.username ?? '',
					passwordEnv: registry?.passwordEnv ?? '',
					plainHttp: registry?.plainHttp ?? false
				};
			});
		}
	});

	const refused = $derived(
		registry || fields.registry.trim() === '' ? null : refuse(fields.registry.trim())
	);
	const login = $derived(fields.username.trim() !== '' || fields.passwordEnv.trim() !== '');

	async function save() {
		const passwordEnv = fields.passwordEnv.trim() || null;
		onsave({
			registry: registry?.registry ?? fields.registry.trim(),
			username: fields.username.trim() || null,
			passwordEnv,
			plainHttp: fields.plainHttp,
			passwordSet: passwordEnv === registry?.passwordEnv ? registry.passwordSet : null
		});
	}
</script>

<FormDialog
	bind:open
	title={registry ? `Edit ${registry.registry}` : 'Add registry'}
	description="Where Corral pulls sandbox images from and reads their sizes. It applies once you save the registries."
	submit={registry ? 'Change registry' : 'Add registry'}
	ready={refused === null}
	onsubmit={save}
>
	{#if !registry}
		<Field.Field data-invalid={refused ? true : undefined}>
			<Field.Label for="registry-host">Registry</Field.Label>
			<Input
				id="registry-host"
				autocomplete="off"
				autocapitalize="none"
				spellcheck={false}
				required
				placeholder="ghcr.io"
				aria-invalid={refused ? true : undefined}
				aria-describedby="registry-host-hint"
				bind:value={fields.registry}
			/>
			{#if refused}
				<Field.Error id="registry-host-hint">{refused}</Field.Error>
			{:else}
				<Field.Description id="registry-host-hint">
					Its host, with its port if it has one, such as registry.example.lan:5000.
				</Field.Description>
			{/if}
		</Field.Field>
	{/if}
	<Field.Field>
		<Field.Label for="registry-username">Username</Field.Label>
		<Input
			id="registry-username"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			required={login}
			aria-describedby="registry-login-hint"
			bind:value={fields.username}
		/>
	</Field.Field>
	<Field.Field>
		<Field.Label for="registry-password-env">Password variable</Field.Label>
		<Input
			id="registry-password-env"
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			required={login}
			placeholder="CORRAL_REGISTRY_TOKEN"
			aria-describedby="registry-login-hint"
			bind:value={fields.passwordEnv}
		/>
		<Field.Description id="registry-login-hint">
			The environment variable that holds the password or token where Corral runs; the secret stays
			there. Leave both empty for a registry that needs no login.
		</Field.Description>
	</Field.Field>
	<Field.Field orientation="horizontal">
		<Checkbox
			id="registry-plain-http"
			aria-describedby="registry-plain-http-hint"
			bind:checked={fields.plainHttp}
		/>
		<Field.Content>
			<Field.Label for="registry-plain-http">Plain HTTP</Field.Label>
			<Field.Description id="registry-plain-http-hint">
				Read image sizes over HTTP, for a registry on your network without TLS.
			</Field.Description>
		</Field.Content>
	</Field.Field>
</FormDialog>
