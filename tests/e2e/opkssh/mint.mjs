// A stand-in OpenID provider for tests/e2e/opkssh.sh, for the certificates a real one will not
// issue: from another issuer, or issued long ago. It builds PK tokens the way opkssh login does
// (openpubkey's nonce commitment to the CIC header), and the certificates with ssh-keygen.
//
//   key <dir>                                   makes the provider's signing key
//   serve <host> <port> <issuer> <dir> <cert> <key>
//                                               serves its discovery document and JWKS over TLS
//   cert <dir> <issuer> <aud> <sub> <age-secs> <out>
//                                               writes <out>, <out>.pub and <out>-cert.pub for a
//                                               token issued <age-secs> ago
//   subject <cert>                              prints the subject of a certificate's token
//   retoken <cert> <key> <out> <claim>          puts the certificate's token, with the numeric
//                                               claim <claim> changed unless it is empty, in a
//                                               certificate for <key> signed by it
import { execFileSync } from 'node:child_process';
import { createHash, createPrivateKey, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:https';
import { join } from 'node:path';

const KID = 'mint';
const [command, ...args] = process.argv.slice(2);

const b64url = (data) => Buffer.from(data).toString('base64url');

/** JSON with sorted keys, as Go's json.Marshal writes the map openpubkey hashes. */
function canonical(value) {
	if (Array.isArray(value)) {
		return `[${value.map(canonical).join(',')}]`;
	}
	if (value && typeof value === 'object') {
		const fields = Object.keys(value)
			.sort()
			.map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`);
		return `{${fields.join(',')}}`;
	}
	return JSON.stringify(value);
}

function key([dir]) {
	const { privateKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
	writeFileSync(join(dir, 'op.pem'), privateKey.export({ type: 'pkcs8', format: 'pem' }));
}

function opKey(dir) {
	return createPrivateKey(readFileSync(join(dir, 'op.pem')));
}

function serve([host, port, issuer, dir, cert, keyFile]) {
	const { kty, n, e } = opKey(dir).export({ format: 'jwk' });
	const jwks = { keys: [{ kty, n, e, kid: KID, alg: 'RS256', use: 'sig' }] };
	const discovery = {
		issuer,
		authorization_endpoint: `${issuer}/authorize`,
		token_endpoint: `${issuer}/token`,
		jwks_uri: `${issuer}/jwks`,
		response_types_supported: ['code'],
		subject_types_supported: ['public'],
		id_token_signing_alg_values_supported: ['RS256']
	};
	const routes = { '/.well-known/openid-configuration': discovery, '/jwks': jwks };
	createServer({ cert: readFileSync(cert), key: readFileSync(keyFile) }, (request, response) => {
		const body = routes[new URL(request.url, issuer).pathname];
		response.writeHead(body ? 200 : 404, { 'content-type': 'application/json' });
		response.end(JSON.stringify(body ?? {}));
	}).listen(Number(port), host);
}

/** Writes the key at `out`, its public half, and a certificate for it carrying `token`. */
function certify(out, privatePem, token) {
	writeFileSync(out, privatePem, { mode: 0o600 });
	writeFileSync(`${out}.pub`, execFileSync('ssh-keygen', ['-y', '-f', out]));
	const extension = `extension:openpubkey-pkt=${token}`;
	execFileSync('ssh-keygen', ['-q', '-s', out, '-I', 'e2e', '-n', 'opkssh-wildcard', '-O', 'clear']
		.concat(['-O', extension, `${out}.pub`]));
}

function cert([dir, issuer, aud, sub, age, out]) {
	const user = generateKeyPairSync('ec', { namedCurve: 'P-256' });
	const { kty, crv, x, y } = user.publicKey.export({ format: 'jwk' });
	const cic = {
		alg: 'ES256',
		rz: randomBytes(32).toString('hex'),
		typ: 'CIC',
		upk: { alg: 'ES256', crv, kty, x, y }
	};
	const cicHeader = canonical(cic);
	const iat = Math.floor(Date.now() / 1000) - Number(age);
	const payload = b64url(
		JSON.stringify({
			iss: issuer,
			aud,
			sub,
			iat,
			exp: iat + 3600,
			nonce: createHash('sha3-256').update(cicHeader).digest('base64url')
		})
	);
	const opHeader = b64url(JSON.stringify({ alg: 'RS256', kid: KID, typ: 'JWT' }));
	const opSignature = sign('sha256', Buffer.from(`${opHeader}.${payload}`), opKey(dir));
	const cicSigned = `${b64url(cicHeader)}.${payload}`;
	const cicSignature = sign('sha256', Buffer.from(cicSigned), {
		key: user.privateKey,
		dsaEncoding: 'ieee-p1363'
	});
	const token = [payload, opHeader, b64url(opSignature), b64url(cicHeader), b64url(cicSignature)];
	certify(out, user.privateKey.export({ type: 'pkcs8', format: 'pem' }), token.join(':'));
}

/** The PK token in a certificate's `openpubkey-pkt` extension. */
function token(certFile) {
	const blob = Buffer.from(readFileSync(certFile, 'utf8').split(/\s+/)[1], 'base64');
	let at = 0;
	const string = () => {
		const length = blob.readUInt32BE(at);
		const value = blob.subarray(at + 4, at + 4 + length);
		at += 4 + length;
		return value;
	};
	const kind = string().toString();
	const fields = kind.startsWith('ssh-ed25519') ? 2 : 3;
	for (let field = 0; field < fields; field++) string();
	at += 12;
	string();
	string();
	at += 16;
	string();
	const extensions = string();
	for (let offset = 0; offset < extensions.length; ) {
		const nameLength = extensions.readUInt32BE(offset);
		const name = extensions.subarray(offset + 4, offset + 4 + nameLength).toString();
		offset += 4 + nameLength;
		const dataLength = extensions.readUInt32BE(offset);
		const data = extensions.subarray(offset + 4, offset + 4 + dataLength);
		offset += 4 + dataLength;
		if (name === 'openpubkey-pkt') {
			return data.subarray(4).toString();
		}
	}
	throw new Error(`${certFile} carries no PK token`);
}

function subject([certFile]) {
	const payload = token(certFile).split(':')[0];
	console.log(JSON.parse(Buffer.from(payload, 'base64url')).sub);
}

function retoken([certFile, keyFile, out, claim]) {
	const parts = token(certFile).split(':');
	if (claim) {
		const claims = JSON.parse(Buffer.from(parts[0], 'base64url'));
		claims[claim] += 1;
		parts[0] = b64url(JSON.stringify(claims));
	}
	certify(out, readFileSync(keyFile), parts.join(':'));
}

const commands = { key, serve, cert, subject, retoken };
commands[command](args);
