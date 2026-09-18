// Servidor RCON falso (protocolo Source) pro teste de paridade.
// Uso: node fake-rcon.mjs <porta> <senha> — responde ao "list" como o Minecraft.
import net from 'node:net';

const [port, password] = [Number(process.argv[2]), process.argv[3]];
const REPLY = 'There are 2 of a max of 10 players online: steve, alex';

function pkt(id, type, body) {
  const b = Buffer.from(body + '\0\0', 'utf8');
  const out = Buffer.alloc(12 + b.length);
  out.writeInt32LE(8 + b.length, 0); out.writeInt32LE(id, 4); out.writeInt32LE(type, 8);
  b.copy(out, 12);
  return out;
}

net.createServer((sock) => {
  let buf = Buffer.alloc(0);
  sock.on('error', () => {});
  sock.on('data', (d) => {
    buf = Buffer.concat([buf, d]);
    while (buf.length >= 4) {
      const len = buf.readInt32LE(0);
      if (buf.length < 4 + len) break;
      const id = buf.readInt32LE(4), type = buf.readInt32LE(8);
      const body = buf.slice(12, 4 + len - 2).toString('utf8');
      buf = buf.slice(4 + len);
      if (type === 3) sock.write(pkt(body === password ? id : -1, 2, ''));
      else if (type === 2) sock.write(pkt(id, 0, body === 'list' ? REPLY : `Unknown command: ${body}`));
    }
  });
}).listen(port, '127.0.0.1', () => console.log(`fake-rcon em 127.0.0.1:${port}`));
