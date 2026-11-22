<script lang="ts" module>
	const day = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' });
	const moment = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
</script>

<script lang="ts">
	let { value }: { value: string } = $props();

	const date = $derived(new Date(value));
</script>

<time class="whitespace-nowrap" datetime={value} title={moment.format(date)}>
	{day.format(date)}
</time>
