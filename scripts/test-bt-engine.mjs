import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, openSync, writeSync, closeSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';
import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'node:http';
import { createSocket } from 'node:dgram';
import { socks5Server } from './socks5-test-server.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const helperArg = process.argv.indexOf('--helper');
const engine = helperArg < 0 ? path.join(root, 'src-tauri/bt-helper/native/ndm-bt-engine.exe') : path.resolve(process.argv[helperArg + 1]);
const seeder = path.join(root, '.native-build/build/bin/ndm-bt-fixture.exe');

class Process {
  constructor(exe, args = []) {
    this.child = spawn(exe, args, { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    this.pending = []; this.events = []; this.stderr = ''; this.exited = false;
    this.child.stderr.on('data', b => { this.stderr += b; if (process.env.NDM_BT_TEST_TRACE) console.log(b.toString()); });
    this.child.stdin.on('error', () => {});
    createInterface({ input: this.child.stdout }).on('line', line => {
      const event = JSON.parse(line);
      if (process.env.NDM_BT_TEST_TRACE) console.log(JSON.stringify({ ...event, metainfo: event.metainfo ? '(cached)' : undefined }));
      if (this.pending.length) this.pending.shift()(event); else this.events.push(event);
    });
    this.done = new Promise(resolve => this.child.on('close', code => { this.exited = true; this.code = code; resolve(code); }));
  }
  send(event) { this.child.stdin.write(JSON.stringify(event) + '\n'); }
  async next(timeout = 15000) {
    if (this.events.length) return this.events.shift();
    if (this.exited) throw Error(`Engine exited ${this.code}: ${this.stderr}`);
    let timer, resolve;
    const event = await Promise.race([
      new Promise(r => { resolve = r; this.pending.push(r); }),
      this.done.then(code => { throw Error(`Engine exited ${code}: ${this.stderr}`); }),
      new Promise((_, reject) => { timer = setTimeout(() => reject(Error('Engine event timed out')), timeout); }),
    ]).finally(() => { clearTimeout(timer); this.pending = this.pending.filter(r => r !== resolve); });
    return event;
  }
  async close() {
    if (!this.exited) this.child.stdin.end();
    let timer;
    await Promise.race([this.done, new Promise(resolve => { timer = setTimeout(() => { this.child.kill(); resolve(); }, 5000); })]);
    clearTimeout(timer);
    if (!this.exited) await this.done;
  }
}

async function fixture(mode, action, tracker) {
  const temp = mkdtempSync(path.join(tmpdir(), 'ndm-native-bt-'));
  const processes = [];
  try {
    const seed = new Process(seeder, [temp, mode, ...(tracker ? [tracker] : [])]); processes.push(seed);
    const info = await seed.next(); assert.equal(info.type, 'fixture');
    assert.equal(info.version, mode === 'v2' ? 2 : 1);
    assert.equal(info.total, 1572924);
    const start = (metainfo = '', limit = 0, directory = path.join(temp, 'download'), changes = {}) => {
      mkdirSync(directory, { recursive: true });
      writeFileSync(path.join(directory, '.ndm-torrent'), info.infoHash);
      const p = new Process(engine); processes.push(p);
      p.send({ type: 'run', url: info.url, hash: info.infoHash, version: info.version,
        directory, metainfo, limit, localTest: true, ...changes });
      return p;
    };
    await action({ temp, info, start });
  } finally {
    await Promise.all(processes.map(p => p.close()));
    rmSync(temp, { recursive: true, force: true });
  }
}

test('BT settings: tracker-only discovery, live reconfiguration, telemetry and supplemental tracker isolation', { timeout: 60000 }, async () => {
  let peerPort = 0;
  const requests = [];
  const tracker = createServer((req, res) => {
    requests.push(req.url);
    if (req.url.startsWith('/fail')) { res.writeHead(500).end(); return; }
    const peer = Buffer.from([127, 0, 0, 1, peerPort >> 8, peerPort & 255]);
    res.end(Buffer.concat([Buffer.from('d8:intervali60e5:peers6:'), peer, Buffer.from('e')]));
  });
  await new Promise(resolve => tracker.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${tracker.address().port}/announce`;
  try {
    await fixture('v2', async ({ info, start }) => {
      peerPort = Number(new URL(info.url).searchParams.get('x.pe').split(':').pop());
      const magnet = new URL(info.url); magnet.searchParams.delete('x.pe');
      const bt = { dht: false, pex: false, lsd: false, upnp: false, bootstrapNodes: [], trackers: [url, url], maxPeers: 23, uploadLimit: 7 };
      const p = start('', 48 * 1024, undefined, { localTest: false, url: magnet.href, bt });
      let cached;
      const progress = await until(p, e => {
        if (e.type === 'metadata') cached = e.metainfo;
        return e.type === 'progress' && e.details?.trackers.some(t => t.state === 'working') && e.downloaded > 0;
      });
      assert.ok(requests.length > 0, 'Tracker must receive an actual announce');
      assert.equal(progress.details.dht, false); assert.equal(progress.details.pex, false);
      assert.equal(progress.details.maxPeers, 23); assert.equal(progress.details.uploadLimit, 7);
      assert.equal(progress.details.trackers.length, 1, 'Configured trackers are deduplicated');
      assert.equal(progress.details.fileProgress.length, info.files.length);
      const inspection = spawnSync(engine, { input: JSON.stringify({ type: 'inspect', metainfo: cached }) + '\n', encoding: 'utf8', windowsHide: true });
      const parsed = JSON.parse(inspection.stdout);
      assert.equal(parsed.fileSizes.reduce((a,b)=>a+b,0), info.total);
      assert.deepEqual(parsed.trackers, [], 'Supplemental trackers must not leak into cached metadata');
      p.send({ type: 'configure', bt: { ...bt, maxPeers: 37, uploadLimit: 11, trackers: [url.replace('/announce','/fail')] } });
      const changed = await until(p, e => e.details?.maxPeers === 37 && e.details.trackers.some(t => t.state === 'error'));
      assert.equal(changed.details.uploadLimit, 11);
      assert.equal(changed.details.trackers.length, 1);
      assert.ok(changed.details.trackers[0].message);
      p.send({ type: 'stop' }); await until(p, e => e.type === 'stopped');
    });
    requests.length = 0;
    await fixture('private', async ({ info, start }) => {
      peerPort = Number(new URL(info.url).searchParams.get('x.pe').split(':').pop());
      const p = start(info.metainfo, 48 * 1024, undefined, { localTest: false, bt: {
        dht: false, pex: true, lsd: false, upnp: false, bootstrapNodes: [], trackers: [url.replace('/announce','/supplemental')], maxPeers: 80, uploadLimit: 0,
      }});
      const progress = await until(p, e => e.details?.private && e.details.trackers.some(t => t.state === 'working'));
      assert.equal(progress.details.dht, false); assert.equal(progress.details.pex, false);
      assert.deepEqual(progress.details.trackers.map(t=>t.url), [url]);
      assert.ok(requests.every(r => !r.startsWith('/supplemental')));
      p.send({ type: 'stop' }); await until(p, e=>e.type==='stopped');
    }, url);
  } finally { tracker.closeAllConnections(); await new Promise(resolve => tracker.close(resolve)); }
});

async function until(p, predicate, authorize = true) {
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    const e = await p.next();
    assert.notEqual(e.type, 'error', e.message);
    if (e.type === 'metadata' && authorize) p.send({ type: 'start' });
    if (predicate(e)) return e;
  }
  throw Error('Transfer deadline exceeded');
}

test('SOCKS5 proxy relays authenticated Peer and Tracker connections with remote DNS', { timeout: 60000 }, async () => {
  const proxy = await socks5Server({ username: 'ndm-user', password: 'ndm-password', resolve: host => host === 'tracker.ndm.test' ? '127.0.0.1' : host });
  let peerPort = 0;
  const tracker = createServer((req, res) => {
    const peer = Buffer.from([127, 0, 0, 1, peerPort >> 8, peerPort & 255]);
    res.end(Buffer.concat([Buffer.from('d8:intervali60e5:peers6:'), peer, Buffer.from('e')]));
  });
  await new Promise(resolve => tracker.listen(0, '127.0.0.1', resolve));
  try {
    await fixture('v2', async ({ temp, info, start }) => {
      peerPort = Number(new URL(info.url).searchParams.get('x.pe').split(':').pop());
      const bt = { dht: false, pex: false, lsd: true, upnp: true, bootstrapNodes: [], maxPeers: 80, uploadLimit: 0,
        trackers: [`http://tracker.ndm.test:${tracker.address().port}/announce`],
        proxy: { enabled: true, host: '127.0.0.1', port: proxy.port, username: 'ndm-user', password: 'ndm-password' } };
      const p = start('', 128 * 1024, undefined, { localTest: false, bt });
      let proxied = false;
      await until(p, event => { if (event.details?.proxy) proxied = true; return event.type === 'complete'; });
      assert.equal(await p.done, 0);
      assert.ok(proxied, 'Runtime must identify proxied sessions');
      assert.ok(proxy.requests.some(r => (r.command === 1 || r.command === 'udp') && r.port === peerPort), 'Peer payload must use SOCKS5 over TCP or uTP');
      assert.ok(proxy.requests.some(r => r.command === 1 && r.host === 'tracker.ndm.test'), 'Tracker hostname must be resolved by the proxy');
      for (const file of info.files) assert.deepEqual(readFileSync(path.join(temp, 'download', file)), readFileSync(path.join(temp, 'seed', file)));
      for (const trackers of [bt.trackers, []]) {
        const wrongAuth = start('', 0, path.join(temp, `wrong-auth-${trackers.length}`), { localTest: false, bt: { ...bt, trackers, proxy: { ...bt.proxy, password: 'wrong' } } });
        let failure; const deadline = Date.now() + 10000;
        do {
          assert.ok(Date.now() < deadline, `Proxy authentication failure must be reported promptly (${trackers.length} Trackers)`);
          failure = await wrongAuth.next();
          assert.notEqual(failure.type, 'metadata', 'Proxy authentication failure must not fall back to direct metadata retrieval');
        } while (failure.type === 'progress');
        assert.equal(failure.type, 'error'); assert.match(failure.message, /SOCKS5 proxy.*authentication/i);
        assert.equal(await wrongAuth.done, 1);
      }
    });
  } finally { await proxy.close(); tracker.closeAllConnections(); await new Promise(resolve => tracker.close(resolve)); }
});

test('SOCKS5 without authentication relays UDP Tracker, DHT and a complete BTv1 download', { timeout: 60000 }, async () => {
  const proxy = await socks5Server(), tracker = createSocket('udp4'), dht = createSocket('udp4');
  let peerPort = 0;
  const trackerSources = [], dhtSources = [];
  tracker.on('message', (packet, from) => {
    if (packet.length < 16) return;
    trackerSources.push(from);
    const action = packet.readUInt32BE(8), transaction = packet.readUInt32BE(12);
    if (action === 0) {
      const reply = Buffer.alloc(16); reply.writeUInt32BE(transaction, 4); reply.writeBigUInt64BE(123n, 8);
      tracker.send(reply, from.port, from.address);
    } else if (action === 1) {
      const reply = Buffer.alloc(26); reply.writeUInt32BE(1, 0); reply.writeUInt32BE(transaction, 4); reply.writeUInt32BE(60, 8); reply.writeUInt32BE(1, 16);
      reply.set([127, 0, 0, 1], 20); reply.writeUInt16BE(peerPort, 24);
      tracker.send(reply, from.port, from.address);
    }
  });
  dht.on('message', (_packet, from) => dhtSources.push(from));
  await Promise.all([new Promise(r => tracker.bind(0, '127.0.0.1', r)), new Promise(r => dht.bind(0, '127.0.0.1', r))]);
  try {
    await fixture('v1', async ({ temp, info, start }) => {
      peerPort = Number(new URL(info.url).searchParams.get('x.pe').split(':').pop());
      const bt = {
        dht: false, pex: false, bootstrapNodes: [`127.0.0.1:${dht.address().port}`],
        trackers: [`udp://127.0.0.1:${tracker.address().port}/announce`],
        proxy: { enabled: true, host: '127.0.0.1', port: proxy.port },
      };
      const p = start('', 128 * 1024, undefined, { localTest: false, bt });
      // Bootstrap DHT once the SOCKS UDP association is established; initial
      // bootstrap retries can otherwise occur after this small file completes.
      await until(p, e => e.type === 'metadata', false);
      p.send({ type: 'configure', bt: { ...bt, dht: true } });
      p.send({ type: 'start' });
      await until(p, e => e.type === 'complete'); assert.equal(await p.done, 0);
      for (const file of info.files) assert.deepEqual(readFileSync(path.join(temp, 'download', file)), readFileSync(path.join(temp, 'seed', file)));
      const relayed = port => proxy.requests.some(r => r.command === 'udp' && r.port === port);
      assert.ok(relayed(tracker.address().port), 'UDP Tracker must use the proxy UDP association');
      assert.ok(relayed(dht.address().port), 'DHT bootstrap packets must use the proxy UDP association');
      assert.ok(trackerSources.length && dhtSources.length);
      assert.ok(trackerSources.every(t => dhtSources.some(d => t.address === d.address && t.port === d.port)), 'Tracker and DHT must receive the same proxy relay source');
    });
  } finally {
    await proxy.close();
    await Promise.all([new Promise(r => tracker.close(r)), new Promise(r => dht.close(r))]);
  }
});

for (const mode of ['v1', 'v2', 'hybrid']) {
  test(`${mode}: real magnet metadata, rate limit, pause, fresh process resume and corrupt piece repair`, { timeout: 60000 }, async () => {
    await fixture(mode, async ({ temp, info, start }) => {
      const first = start('', 128 * 1024); let cached, paused = false;
      await until(first, e => {
        if (e.type === 'metadata') cached = e.metainfo;
        if (e.type === 'progress' && e.downloaded >= 65536 && !paused) { first.send({ type: 'stop' }); paused = true; }
        assert.notEqual(e.type, 'complete', 'rate limited task must pause before completion');
        return e.type === 'stopped';
      });
      assert.ok(paused && cached); assert.equal(await first.done, 0);
      const directory = path.join(temp, 'download');
      const fd = openSync(path.join(directory, 'pack/a.bin'), 'r+');
      writeSync(fd, Buffer.alloc(32768, 255), 0, 32768, 0); closeSync(fd);
      const resumed = start(cached, 128 * 1024);
      resumed.send({ type: 'limit', bps: 0 });
      await until(resumed, e => e.type === 'complete'); assert.equal(await resumed.done, 0);
      for (const file of info.files) assert.deepEqual(readFileSync(path.join(directory, file)), readFileSync(path.join(temp, 'seed', file)));
      assert.ok(cached.length > 100);
    });
  });

  test(`${mode}: torrent import, content authorization, file transfer and parent EOF`, { timeout: 60000 }, async () => {
    await fixture(mode, async ({ temp, info, start }) => {
      const inspection = spawnSync(engine, { input: JSON.stringify({ type: 'inspect', metainfo: info.metainfo }) + '\n', encoding: 'utf8', windowsHide: true, timeout: 15000 });
      assert.equal(inspection.status, 0, inspection.stdout + inspection.stderr);
      const parsed = JSON.parse(inspection.stdout);
      assert.equal(parsed.infoHash, info.infoHash); assert.equal(parsed.total, info.total);
      assert.deepEqual(parsed.files, info.files);
      const p = start(info.metainfo);
      await until(p, e => e.type === 'metadata', false);
      await new Promise(resolve => setTimeout(resolve, 400));
      // No content may transfer while the parent is still checking paths.
      let event;
      do { event = await p.next(); assert.notEqual(event.type, 'error', event.message); } while (event.type !== 'progress');
      assert.equal(event.downloaded, 0);
      p.send({ type: 'start' }); await until(p, e => e.type === 'complete'); assert.equal(await p.done, 0);
      for (const file of info.files) assert.deepEqual(readFileSync(path.join(temp, 'download', file)), readFileSync(path.join(temp, 'seed', file)));
      const pending = start('', 0, path.join(temp, 'cancel'), { url: `magnet:?xt=urn:${info.version === 2 ? 'btmh:1220' : 'btih:'}${'1'.repeat(info.version === 2 ? 64 : 40)}`, hash: '1'.repeat(info.version === 2 ? 64 : 40) });
      pending.child.stdin.end(); await until(pending, e => e.type === 'stopped', false); assert.equal(await pending.done, 0);
    });
  });
}

test('rejects malformed metainfo and mismatched cached hashes', { timeout: 20000 }, async () => {
  const result = spawnSync(engine, { input: JSON.stringify({ type: 'inspect', metainfo: Buffer.from('not a torrent').toString('base64') }) + '\n', encoding: 'utf8', timeout: 15000, windowsHide: true });
  assert.equal(result.status, 1); assert.equal(JSON.parse(result.stdout).type, 'error');
  await fixture('v2', async ({ info, start }) => {
    const p = start(info.metainfo, 0, undefined, { hash: '0'.repeat(64) });
    const event = await p.next(); assert.equal(event.type, 'error'); assert.match(event.message, /hash mismatch/);
    assert.equal(await p.done, 1);
  });
});

test('rejects output directory junctions before opening torrent content', { timeout: 20000 }, async () => {
  await fixture('v2', async ({ temp, info, start }) => {
    const target = path.join(temp, 'unrelated'); mkdirSync(target);
    const link = path.join(temp, 'linked'); symlinkSync(target, link, 'junction');
    const p = start(info.metainfo, 0, link);
    const event = await p.next(); assert.equal(event.type, 'error'); assert.match(event.message, /link|junction/);
    assert.equal(await p.done, 1);
  });
});

// Retain binary dictionary keys so hashing/re-encoding v2 Merkle layers is exact.
function bdecode(buffer) {
  let pos = 0;
  function read() {
    const token = buffer[pos];
    if (token === 105) { const end = buffer.indexOf(101, ++pos); const n = Number(buffer.subarray(pos, end)); pos = end + 1; return n; }
    if (token === 100 || token === 108) {
      ++pos; const value = token === 100 ? new Map() : [];
      while (buffer[pos] !== 101) { if (value instanceof Map) { const key = read(); value.set(key, read()); } else value.push(read()); }
      ++pos; return value;
    }
    const colon = buffer.indexOf(58, pos); const size = Number(buffer.subarray(pos, colon));
    pos = colon + 1; const bytes = buffer.subarray(pos, pos + size); pos += size; return bytes;
  }
  return read();
}
function bencode(value) {
  if (Buffer.isBuffer(value)) return Buffer.concat([Buffer.from(`${value.length}:`), value]);
  if (typeof value === 'number') return Buffer.from(`i${value}e`);
  if (Array.isArray(value)) return Buffer.concat([Buffer.from('l'), ...value.map(bencode), Buffer.from('e')]);
  return Buffer.concat([Buffer.from('d'), ...[...value.entries()].sort(([a], [b]) => Buffer.compare(a, b)).flatMap(([k, v]) => [bencode(k), bencode(v)]), Buffer.from('e')]);
}
function field(dict, name) { return [...dict.entries()].find(([k]) => k.toString() === name)?.[1]; }

test('rejects invalid BTv2 Merkle piece layers', { timeout: 20000 }, async () => {
  const value = bdecode(readFileSync(path.join(root, 'src-tauri/tests/fixtures/bt/v2.torrent')));
  const layers = field(value, 'piece layers'); assert.ok(layers.size);
  for (const [key, hashes] of layers) layers.set(key, Buffer.alloc(hashes.length));
  const result = spawnSync(engine, { input: JSON.stringify({ type: 'inspect', metainfo: bencode(value).toString('base64') }) + '\n', encoding: 'utf8', timeout: 15000, windowsHide: true });
  assert.equal(result.status, 1); assert.match(JSON.parse(result.stdout).message, /piece layer/);
});

test('hybrid metadata obtained through a v2-only magnet retains the requested v2 identity', { timeout: 30000 }, async () => {
  await fixture('hybrid', async ({ temp, info, start }) => {
    const metadata = bdecode(Buffer.from(info.metainfo, 'base64'));
    const hash = createHash('sha256').update(bencode(field(metadata, 'info'))).digest('hex');
    const url = info.url.replace(/xt=urn:btih:[^&]*&?/, '');
    assert.ok(url.includes(`urn:btmh:1220${hash}`));
    const directory = path.join(temp, 'v2-hybrid');
    const p = start('', 0, directory, { hash, version: 2, url });
    writeFileSync(path.join(directory, '.ndm-torrent'), hash);
    await until(p, e => e.type === 'complete'); assert.equal(await p.done, 0);
    for (const file of info.files) assert.deepEqual(readFileSync(path.join(directory, file)), readFileSync(path.join(temp, 'seed', file)));
  });
});
