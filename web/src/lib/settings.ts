import type {
	CatalogChange,
	CatalogSettings,
	LimitsChange,
	LimitsSettings,
	LockedNumber,
	RegistriesSettings,
	RegistryChange,
	SandboxDefaults,
	Source
} from '#lib/api/generated/index.js';

export type ImageEntry = CatalogChange['images'][number];
export type SizeEntry = CatalogChange['sizes'][number];
export type RegistryEntry = RegistryChange & {
	/** Whether the password variable is set where Corral runs, unknown until saved. */
	passwordSet: boolean | null;
};

/**
 * A row of a settings list, keyed by its image or size key or its registry: one the config file
 * sets, locked; one kept here, which the admin changes; or one kept here whose key the config file
 * now sets too, so it is not used and can only be removed.
 */
export type Row<T> =
	{ state: 'locked' | 'editable'; key: string; entry: T } | { state: 'shadowed'; key: string };

/** A value, and whether the config file sets it. */
export type Locked<T> = { value: T; locked: boolean };

export type NamedImagesForm = {
	allowed: boolean;
	/** One per line. */
	registries: string;
	maxSize: string;
};

export type CatalogView = {
	images: Row<ImageEntry>[];
	sizes: Row<SizeEntry>[];
	defaults: Locked<SandboxDefaults>;
	/** A saved default is no longer offered, so the first entry offered stands in. */
	defaultsFellBack: boolean;
	namedImages: Locked<NamedImagesForm>;
};

export type LimitsView = {
	maxSandboxesPerMember: Locked<number | null>;
	maxRunning: Locked<number | null>;
};

export type RegistriesView = { registries: Row<RegistryEntry>[] };

/** Catalog keys, as an input's `pattern`. */
export const KEY_PATTERN = '[a-z0-9\\-]{1,32}';
const NUMBER = '(?:[0-9]+(?:\\.[0-9]*)?|\\.[0-9]+)';
/** CPU quantities as Corral reads them, such as `2`, `1.5` or `500m`, as an input's `pattern`. */
export const CPU_PATTERN = `${NUMBER}m?`;
/** Byte quantities as Corral reads them, such as `4Gi`, `512Mi` or `1G`, as an input's `pattern`. */
export const BYTES_PATTERN = `${NUMBER}(?:[KMGTPE]i|[kMGTPE])?`;

export function catalogView(settings: CatalogSettings): CatalogView {
	const named = settings.namedImages;
	return {
		images: [
			...settings.images.map((image) => row(image.key, image)),
			...settings.shadowedImages.map(shadowed)
		],
		sizes: [
			...settings.sizes.map((size) => row(size.key, size)),
			...settings.shadowedSizes.map(shadowed)
		],
		defaults: {
			value: { image: settings.defaultImage, size: settings.defaultSize },
			locked: settings.defaultsSource === 'config'
		},
		defaultsFellBack: settings.defaultsFellBack,
		namedImages: {
			value: {
				allowed: named !== null,
				registries: named?.registries.join('\n') ?? '',
				maxSize: named?.maxSize ?? ''
			},
			locked: settings.namedImagesSource === 'config'
		}
	};
}

export function catalogChange(view: CatalogView): CatalogChange {
	const named = unlocked(view.namedImages);
	return {
		images: kept(view.images),
		sizes: kept(view.sizes),
		defaults: unlocked(view.defaults),
		namedImages: named?.allowed
			? {
					registries: named.registries
						.split('\n')
						.map((line) => line.trim())
						.filter(Boolean),
					maxSize: named.maxSize.trim()
				}
			: null
	};
}

export function limitsView(settings: LimitsSettings): LimitsView {
	return {
		maxSandboxesPerMember: lockedNumber(settings.maxSandboxesPerMember),
		maxRunning: lockedNumber(settings.maxRunning)
	};
}

export function limitsChange(view: LimitsView): LimitsChange {
	return {
		maxSandboxesPerMember: unlocked(view.maxSandboxesPerMember),
		maxRunning: unlocked(view.maxRunning)
	};
}

export function registriesView(settings: RegistriesSettings): RegistriesView {
	return {
		registries: [
			...settings.registries.map((registry) => row(registry.registry, registry)),
			...settings.shadowed.map(shadowed)
		]
	};
}

export function registriesChange(view: RegistriesView): RegistryChange[] {
	return kept(view.registries).map(({ passwordSet, ...registry }) => registry);
}

/** Why `key` cannot name a new row of `rows`, if it cannot, with `taken` such as "a size". */
export function refusal(rows: Row<unknown>[], key: string, taken: string): string | null {
	const row = rows.find((each) => each.key.toLowerCase() === key.trim().toLowerCase());
	if (!row) {
		return null;
	}
	return row.state === 'editable' ? `${key} is already ${taken}` : `${key} is set in config`;
}

/** `rows` with `entry` kept here under `key`, in place of the row it had or after the others. */
export function put<T>(rows: Row<T>[], key: string, entry: T): Row<T>[] {
	const kept: Row<T> = { state: 'editable', key, entry };
	const at = rows.findIndex((row) => row.state === 'editable' && row.key === key);
	return at === -1 ? [...rows, kept] : rows.with(at, kept);
}

/** `rows` without the row kept here under `key`. */
export function without<T>(rows: Row<T>[], key: string): Row<T>[] {
	return rows.filter((row) => row.state === 'locked' || row.key !== key);
}

function row<T extends { source: Source }>(key: string, { source, ...entry }: T) {
	const state = source === 'config' ? ('locked' as const) : ('editable' as const);
	return { state, key, entry };
}

function shadowed(key: string) {
	return { state: 'shadowed' as const, key };
}

/** The entries a save sends: the config file's are not the database's, and a shadowed one is refused. */
function kept<T>(rows: Row<T>[]): T[] {
	return rows.flatMap((row) => (row.state === 'editable' ? [row.entry] : []));
}

function lockedNumber({ value, source }: LockedNumber): Locked<number | null> {
	return { value, locked: source === 'config' };
}

/** The value a save sends, or `null` to leave it to the config file. */
function unlocked<T>({ value, locked }: Locked<T>): T | null {
	return locked ? null : value;
}
