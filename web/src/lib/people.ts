import type {
	Handle,
	LocalAccount,
	PictureUrl,
	Provider,
	Role,
	User,
	UserStatus
} from '#lib/api/generated/index.js';

/** What the People page lists, fetched together. */
export type Directory = {
	users: User[];
	/** Empty when no provider keeps accounts. */
	accounts: LocalAccount[];
	/** How many sandboxes someone without an allowance may have. */
	limit: number;
};

type Shown = {
	handle: Handle;
	picture: PictureUrl | null;
	email: string | null;
	role: Role;
	/** The label of the provider they sign in with. */
	signsInWith: string;
	/** Their allowance, else the limit. */
	maxSandboxes: number;
	createdAt: string;
};

/** A row of the People page. */
export type Person = Shown &
	(
		| {
				user: User;
				/** The local account they sign in with, if they sign in with one. */
				account: LocalAccount | null;
				status: UserStatus;
				/** Whether they are the admin looking at the page. */
				self: boolean;
		  }
		| {
				/** A local account no one has signed in with yet. */
				user: null;
				account: LocalAccount;
				status: 'waiting';
				self: false;
		  }
	);

/**
 * Everyone in `directory`: each user, with the local account they sign in with, then the local
 * accounts no one has signed in with yet.
 */
export function people(
	{ users, accounts, limit }: Directory,
	providers: Provider[],
	self: Handle
): Person[] {
	const keeper = providers.find((provider) => provider.accounts !== null);
	const label = (id: string) => providers.find((provider) => provider.id === id)?.label ?? id;
	const byUsername = new Map(accounts.map((account) => [account.username, account]));
	const claimed = new Set(
		users.filter((user) => user.provider === keeper?.id).map((user) => user.handle)
	);

	const signedIn = users.map((user): Person => ({
		handle: user.handle,
		picture: user.picture,
		email: user.email,
		role: user.role,
		signsInWith: label(user.provider),
		maxSandboxes: user.allowance?.maxSandboxes ?? limit,
		createdAt: user.createdAt,
		user,
		account: user.provider === keeper?.id ? (byUsername.get(user.handle) ?? null) : null,
		status: user.status,
		self: user.handle === self
	}));
	const waiting = keeper
		? accounts
				.filter((account) => !claimed.has(account.username))
				.map((account): Person => ({
					handle: account.username,
					picture: null,
					email: null,
					role: account.role,
					signsInWith: keeper.label,
					maxSandboxes: limit,
					createdAt: account.createdAt,
					user: null,
					account,
					status: 'waiting',
					self: false
				}))
		: [];
	return [...signedIn, ...waiting];
}
