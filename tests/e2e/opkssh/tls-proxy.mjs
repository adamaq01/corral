// Serves VoidAuth, which speaks plain HTTP, over TLS for tests/e2e/opkssh.sh: opkssh only takes
// https:// issuers. VoidAuth trusts X-Forwarded-Proto from its peer.
//
//   node tls-proxy.mjs <host> <port> <upstream-port> <cert> <key>
import { request } from 'node:http';
import { createServer } from 'node:https';
import { readFileSync } from 'node:fs';

const [host, port, upstream, cert, key] = process.argv.slice(2);
createServer({ cert: readFileSync(cert), key: readFileSync(key) }, (incoming, outgoing) => {
	const forwarded = request(
		{
			host: '127.0.0.1',
			port: Number(upstream),
			method: incoming.method,
			path: incoming.url,
			headers: { ...incoming.headers, 'x-forwarded-proto': 'https' }
		},
		(answer) => {
			outgoing.writeHead(answer.statusCode, answer.headers);
			answer.pipe(outgoing);
		}
	);
	forwarded.on('error', (error) => {
		outgoing.writeHead(502);
		outgoing.end(String(error));
	});
	incoming.pipe(forwarded);
}).listen(Number(port), host);
