// Local SOCKS5 fixture. Exercises actual proxy handshakes and relayed traffic.
import net from 'node:net';
import dgram from 'node:dgram';

export async function socks5Server({ username = '', password = '', resolve = host => host } = {}) {
  const sockets = new Set(), datagrams = new Set(), requests = [];
  const server = net.createServer(socket => {
    sockets.add(socket); socket.on('close', () => sockets.delete(socket)); socket.on('error', () => {});
    let buffer = Buffer.alloc(0), state = 'greeting';
    const onData = chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (!socket.destroyed) {
        if (state === 'greeting') {
          if (buffer.length < 2 || buffer.length < 2 + buffer[1]) return;
          const methods = buffer.subarray(2, 2 + buffer[1]);
          const method = username ? 2 : 0;
          if (buffer[0] !== 5 || !methods.includes(method)) { socket.end(Buffer.from([5, 255])); return; }
          buffer = buffer.subarray(2 + buffer[1]); socket.write(Buffer.from([5, method])); state = username ? 'auth' : 'request';
        } else if (state === 'auth') {
          if (buffer.length < 2 || buffer.length < 3 + buffer[1]) return;
          const userEnd = 2 + buffer[1], end = userEnd + 1 + buffer[userEnd];
          if (buffer.length < end) return;
          const valid = buffer.subarray(2, userEnd).toString() === username && buffer.subarray(userEnd + 1, end).toString() === password;
          socket.write(Buffer.from([1, valid ? 0 : 1])); if (!valid) { socket.end(); return; }
          buffer = buffer.subarray(end); state = 'request';
        } else if (state === 'request') {
          if (buffer.length < 5) return;
          const type = buffer[3], length = type === 1 ? 4 : type === 3 ? 1 + buffer[4] : type === 4 ? 16 : -1;
          if (length < 0) { socket.destroy(); return; }
          const end = 4 + length + 2; if (buffer.length < end) return;
          const host = type === 1 ? [...buffer.subarray(4, 8)].join('.') : type === 3 ? buffer.subarray(5, 4 + length).toString() : [...Array(8)].map((_, i) => buffer.readUInt16BE(4 + i * 2).toString(16)).join(':');
          const port = buffer.readUInt16BE(end - 2), command = buffer[1];
          requests.push({ command, host, port });
          if (process.env.NDM_BT_TEST_TRACE) console.log('SOCKS request', { command, host, port });
          const remaining = buffer.subarray(end); buffer = Buffer.alloc(0); state = 'relay';
          socket.removeListener('data', onData);
          if (command === 3) {
            const udp = dgram.createSocket('udp4'); datagrams.add(udp); udp.on('error', () => {});
            const clients = new Map();
            udp.on('message', (packet, from) => {
              const client = clients.get(`${from.address}:${from.port}`);
              if (client) {
                const header = Buffer.from([0, 0, 0, 1, ...from.address.split('.').map(Number), 0, 0]); header.writeUInt16BE(from.port, 8);
                udp.send(Buffer.concat([header, packet]), client.port, client.address);
              } else if (packet.length >= 10 && packet[0] === 0 && packet[1] === 0 && packet[2] === 0 && [1, 3].includes(packet[3])) {
                const type = packet[3], length = type === 1 ? 4 : 1 + packet[4], end = 4 + length + 2;
                if (packet.length < end) return;
                const host = type === 1 ? [...packet.subarray(4, 8)].join('.') : packet.subarray(5, 4 + length).toString();
                const port = packet.readUInt16BE(end - 2), target = resolve(host);
                if (!port) return;
                requests.push({ command: 'udp', host, port }); clients.set(`${target}:${port}`, from);
                udp.send(packet.subarray(end), port, target);
              }
            });
            udp.bind(0, '127.0.0.1', () => {
              const reply = Buffer.from([5, 0, 0, 1, 127, 0, 0, 1, 0, 0]); reply.writeUInt16BE(udp.address().port, 8); socket.write(reply);
            });
            socket.once('close', () => { datagrams.delete(udp); udp.close(); });
          } else if (command === 1) {
            const upstream = net.connect({ host: resolve(host), port }); sockets.add(upstream);
            upstream.on('close', () => sockets.delete(upstream)); upstream.on('error', error => { if (process.env.NDM_BT_TEST_TRACE) console.log('SOCKS upstream', error.message); socket.destroy(); });
            upstream.once('connect', () => {
              socket.write(Buffer.from([5, 0, 0, 1, 127, 0, 0, 1, 0, 0]));
              if (remaining.length) upstream.write(remaining);
              socket.pipe(upstream); upstream.pipe(socket);
            });
            socket.once('close', () => upstream.destroy()); upstream.once('close', () => socket.destroy());
          } else socket.end(Buffer.from([5, 7, 0, 1, 0, 0, 0, 0, 0, 0]));
          return;
        } else return;
      }
    };
    socket.on('data', onData);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return {
    port: server.address().port, requests,
    async close() {
      for (const socket of sockets) socket.destroy();
      await new Promise(resolve => server.close(resolve));
      for (const udp of datagrams) { try { udp.close(); } catch {} }
    },
  };
}
