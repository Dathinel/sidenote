// Loopback-only opaque forwarding fixture. Does not qualify the Cloudflare relay.
const {WebSocketServer, WebSocket} = require('ws');
const crypto = require('node:crypto');
const server = new WebSocketServer({host: '127.0.0.1', port: Number(process.argv[2] || 0), maxPayload: 128 * 1024});
const peers = new Map();
const send = (socket, value) => { if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(value)); };
const roomPeers = state => [...peers].filter(([, other]) => other.room === state.room && other.kind === state.kind);
server.on('listening', () => console.log(JSON.stringify({port: server.address().port})));
server.on('connection', (socket, request) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  const match = /^\/(pair|sync)\/([a-f0-9]{64})$/.exec(url.pathname);
  if (!match) return socket.close(1008);
  const state = {kind: match[1], room: match[2], role: url.searchParams.get('role'), nonce: crypto.randomBytes(32).toString('hex')};
  send(socket, {type: 'challenge', nonce: state.nonce});
  const deadline = setTimeout(() => socket.close(1008), 10000);
  socket.on('message', (bytes, binary) => {
    try {
      if (!peers.has(socket)) {
        if (binary) throw Error();
        const auth = JSON.parse(bytes.toString());
        if (auth.type !== 'auth' || !/^[a-f0-9]{64}$/.test(auth.key) || !/^[a-f0-9]{128}$/.test(auth.signature)) throw Error();
        const key = crypto.createPublicKey({key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(auth.key, 'hex')]), format: 'der', type: 'spki'});
        if (!crypto.verify(null, Buffer.from(`sidenote-auth-v1:${state.room}:${state.role}:${state.nonce}`), key, Buffer.from(auth.signature, 'hex'))) throw Error();
        state.key = auth.key;
        const others = roomPeers(state);
        if (others.length >= 8 || others.some(([, other]) => other.key === state.key)) throw Error();
        peers.set(socket, state); clearTimeout(deadline);
        send(socket, {type: 'ready', peers: others.map(([, other]) => other.key)});
        for (const [other] of others) send(other, {type: 'joined', key: state.key});
      } else if (!binary) {
        const message = JSON.parse(bytes.toString());
        if (message.type === 'ping') return send(socket, {type: 'pong'});
        if (state.kind !== 'pair' || state.role !== 'owner') throw Error();
        if (message.type === 'grant') send(socket, {type: 'grant-ready'});
        else if (message.type === 'retry') {
          for (const [other, p] of roomPeers(state)) if (p.role === 'guest') other.close();
          send(socket, {type: 'retry-ready'});
        } else if (message.type === 'complete') {
          for (const [other] of roomPeers(state)) other.close();
        } else throw Error();
      } else {
        for (const [other, p] of roomPeers(state)) {
          if (other === socket) continue;
          if (state.kind === 'sync' && (bytes.length < 162 || bytes.subarray(2, 34).toString('hex') !== state.key || bytes.subarray(34, 66).toString('hex') !== p.key)) continue;
          other.send(bytes, {binary: true});
        }
      }
    } catch { socket.close(1008); }
  });
  socket.on('error', () => {});
  socket.on('close', () => {
    clearTimeout(deadline);
    if (peers.delete(socket)) for (const [other] of roomPeers(state)) send(other, {type: 'left', key: state.key});
  });
});
server.on('error', () => { console.error('loopback fixture unavailable'); process.exitCode = 2; });
