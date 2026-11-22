<script lang="ts">
	import { untrack } from 'svelte';
	import { keys } from '#lib/api/keys.js';
	import FormDialog from '#lib/components/form-dialog.svelte';
	import * as Field from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';

	let { open = $bindable(), onAdded }: { open: boolean; onAdded: () => Promise<void> } = $props();

	let label = $state('');
	let publicKey = $state('');
	let labelEdited = false;

	$effect(() => {
		if (open) {
			untrack(() => {
				label = '';
				publicKey = '';
				labelEdited = false;
			});
		}
	});

	/** Until the label is typed in, it follows the pasted key's comment, such as `ada@laptop`. */
	function pasted() {
		if (!labelEdited) {
			label = publicKey.trim().split(/\s+/).slice(2).join(' ');
		}
	}

	async function add() {
		await keys.add({ label: label.trim(), publicKey: publicKey.trim() });
		await onAdded();
	}
</script>

<FormDialog
	bind:open
	title="Add an SSH key"
	description="Paste a public key, such as the contents of ~/.ssh/id_ed25519.pub. Every sandbox you own accepts it."
	submit="Add key"
	onsubmit={add}
>
	<Field.Field>
		<Field.Label for="key-public">Public key</Field.Label>
		<Textarea
			id="key-public"
			bind:value={publicKey}
			oninput={pasted}
			required
			rows={4}
			class="font-mono text-xs break-all"
			placeholder="ssh-ed25519 AAAA… you@laptop"
			spellcheck={false}
		/>
	</Field.Field>
	<Field.Field>
		<Field.Label for="key-label">Label</Field.Label>
		<Input
			id="key-label"
			bind:value={label}
			oninput={() => (labelEdited = true)}
			required
			autocomplete="off"
		/>
		<Field.Description>
			Where the key lives, so you know which one to remove later.
		</Field.Description>
	</Field.Field>
</FormDialog>
