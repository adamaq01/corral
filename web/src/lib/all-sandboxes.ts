import type { Handle, OwnedSandbox } from '#lib/api/generated/index.js';
import { states, type Power } from '#lib/sandbox-status.js';

const OWNER = 'owner';

/** All sandboxes showing those of `owner`, or with `null` everyone's. */
export function sandboxesOf(owner: Handle | null): string {
	return owner === null
		? '/admin/sandboxes'
		: `/admin/sandboxes?${new URLSearchParams({ [OWNER]: owner })}`;
}

/** The owner whose sandboxes `url` shows, or `null` for everyone's. */
export function ownerOf(url: { searchParams: Pick<URLSearchParams, 'get'> }): Handle | null {
	return url.searchParams.get(OWNER) || null;
}

/** A power the admin can use on anyone's sandbox. */
export type AdminPower = Exclude<Power, 'wake'>;

/** A row of All sandboxes. */
export type Row = {
	sandbox: OwnedSandbox;
	/** `<name>.<handle>`, unique across owners, as the admin's texts name it. */
	fullName: string;
	/** What the admin can do to its power from here. */
	powers: AdminPower[];
	deletable: boolean;
};

/** What All sandboxes shows. */
export type Shown = {
	/** The owners to filter by: everyone with a sandbox, and the owner filtered by. */
	owners: Handle[];
	/** The sandboxes of `owner`, or everyone's; `undefined` while they load. */
	rows: Row[] | undefined;
};

/** What All sandboxes shows of `sandboxes` for the owner filtered by. */
export function shown(sandboxes: OwnedSandbox[] | undefined, owner: Handle | null): Shown {
	const owners = new Set(sandboxes?.map((sandbox) => sandbox.owner.handle));
	if (owner !== null) {
		owners.add(owner);
	}
	return {
		owners: [...owners].sort(),
		rows: sandboxes
			?.filter((sandbox) => owner === null || sandbox.owner.handle === owner)
			.map((sandbox) => ({
				sandbox,
				fullName: `${sandbox.name}.${sandbox.owner.handle}`,
				powers: states[sandbox.state].powers.filter(
					(power): power is AdminPower => power !== 'wake'
				),
				deletable: states[sandbox.state].manageable
			}))
	};
}
