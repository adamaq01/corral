<script lang="ts" module>
	const moment = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
</script>

<script lang="ts">
	let { value }: { value: string } = $props();
</script>

<time class="whitespace-nowrap" datetime={value}>{moment.format(new Date(value))}</time>
