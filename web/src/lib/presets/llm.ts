import type { SetupEntry, SharedEntry } from '../api/generated/index.js';
import { own, ownText, put, type Change } from './change.js';

export type Provider = 'anthropic' | 'openai';

export const PROVIDERS: Provider[] = ['anthropic', 'openai'];

/**
 * The variables of each provider: its address, its key, and for Anthropic the bearer token a
 * gateway may take instead, written with the same key when there is an address.
 */
export const NAMES: Record<Provider, { label: string; url: string; key: string; bearer?: string }> =
	{
		anthropic: {
			label: 'Anthropic',
			url: 'ANTHROPIC_BASE_URL',
			key: 'ANTHROPIC_API_KEY',
			bearer: 'ANTHROPIC_AUTH_TOKEN'
		},
		openai: { label: 'OpenAI', url: 'OPENAI_BASE_URL', key: 'OPENAI_API_KEY' }
	};

/**
 * Each provider's address and whether the admin locks it, whether the member set a key, and
 * whether the admin set one for every sandbox, which the member's form then leaves alone.
 */
export type LlmState = {
	rows: Record<Provider, { url: string; urlLocked: boolean; keySet: boolean; keyLocked: boolean }>;
};

/**
 * What the form saves for each provider: the address, and the key, `null` to keep the one set
 * and `''` for none.
 */
export type LlmForm = { rows: Record<Provider, { url: string; key: string | null }> };

const variable = (target: string, secret: boolean) => ({
	kind: 'variable' as const,
	target,
	secret
});

function keyNames(provider: Provider, withUrl: boolean): string[] {
	const { key, bearer } = NAMES[provider];
	return withUrl && bearer ? [key, bearer] : [key];
}

function allKeyNames(provider: Provider): string[] {
	return keyNames(provider, true);
}

export function readLlm(entries: SetupEntry[]): LlmState {
	const required = (name: string) =>
		entries.find(
			(entry) => entry.shared === 'required' && entry.kind === 'variable' && entry.target === name
		);
	const row = (provider: Provider) => {
		const { url } = NAMES[provider];
		const shared = required(url);
		const lockedUrl = shared?.value.shown === 'text' ? shared.value.text : '';
		return {
			url: shared ? lockedUrl : (ownText(entries, variable(url, false)) ?? ''),
			urlLocked: shared !== undefined,
			keySet: allKeyNames(provider).some((name) => own(entries, variable(name, true))),
			keyLocked: allKeyNames(provider).some((name) => required(name) !== undefined)
		};
	};
	return { rows: { anthropic: row('anthropic'), openai: row('openai') } };
}

/** The address the provider's entries follow: the admin's when locked, else the form's. */
function address(state: LlmState, form: LlmForm, provider: Provider): string {
	const read = state.rows[provider];
	return read.urlLocked ? read.url : form.rows[provider].url.trim();
}

export function writeLlm(current: SetupEntry[], form: LlmForm): Change[] {
	const state = readLlm(current);
	return PROVIDERS.flatMap((provider) => {
		const { key } = form.rows[provider];
		const url = address(state, form, provider);
		const needed = keyNames(provider, url !== '');
		const names = state.rows[provider].keyLocked ? [] : allKeyNames(provider);
		const keys = names.flatMap((name) => {
			if (key === null && needed.includes(name)) {
				return [];
			}
			return put(current, variable(name, true), key && needed.includes(name) ? key : null);
		});
		const locked = state.rows[provider].urlLocked;
		return [
			...(locked ? [] : put(current, variable(NAMES[provider].url, false), url || null)),
			...keys
		];
	});
}

/**
 * For each provider, whether its kept key must be typed again: the form's address calls for a
 * variable the key is not yet in, and a key's value never shows to copy it.
 */
export function needsKey(current: SetupEntry[], form: LlmForm): Record<Provider, boolean> {
	const state = readLlm(current);
	const needs = (provider: Provider) =>
		form.rows[provider].key === null &&
		state.rows[provider].keySet &&
		!state.rows[provider].keyLocked &&
		keyNames(provider, address(state, form, provider) !== '').some(
			(name) => !own(current, variable(name, true))
		);
	return { anthropic: needs('anthropic'), openai: needs('openai') };
}

/**
 * One line saying which providers have a key, the admin's or the member's, and through what, or
 * `null` when none does.
 */
export function llmSummary(state: LlmState): string | null {
	const rows = state.rows;
	const keyed = PROVIDERS.filter((provider) => rows[provider].keySet || rows[provider].keyLocked);
	const said = keyed.map((provider) => {
		const { label } = NAMES[provider];
		if (rows[provider].keyLocked) {
			return `${label} key set by the admin`;
		}
		return rows[provider].url ? `${label} through a gateway` : `${label} key set`;
	});
	return said.length > 0 ? said.join(', ') : null;
}

/** The field of the form holding the variable `message` names, as Corral's refusals name one. */
export function fieldNamed(message: string): { provider: Provider; field: 'url' | 'key' } | null {
	const named = (name: string) => new RegExp(`\\b${name}\\b`).test(message);
	for (const provider of PROVIDERS) {
		if (named(NAMES[provider].url)) {
			return { provider, field: 'url' };
		}
		if (allKeyNames(provider).some(named)) {
			return { provider, field: 'key' };
		}
	}
	return null;
}

/** The address the admin requires for each provider, `''` for none. */
export type SharedLlm = Record<Provider, string>;

/** The required shared entry holding the provider's address, which the admin's preset writes. */
export function sharedUrl(shared: SharedEntry[], provider: Provider): SharedEntry | undefined {
	return shared.find(
		(entry) =>
			entry.shared === 'required' &&
			entry.kind === 'variable' &&
			entry.target === NAMES[provider].url
	);
}

export function readSharedLlm(shared: SharedEntry[]): SharedLlm {
	const url = (provider: Provider) => {
		const value = sharedUrl(shared, provider)?.value;
		return value?.shown === 'text' ? value.text : '';
	};
	return { anthropic: url('anthropic'), openai: url('openai') };
}

/** One line naming each provider's shared address, or `null` when none has one. */
export function sharedLlmSummary(urls: SharedLlm): string | null {
	const said = PROVIDERS.filter((provider) => urls[provider] !== '').map(
		(provider) => `${NAMES[provider].label} through ${urls[provider]}`
	);
	return said.length > 0 ? said.join(', ') : null;
}
