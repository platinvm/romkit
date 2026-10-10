// A minimal zip writer: stored entries (assets are PNG, WAV, and JSON, which gain little from
// deflate), UTF-8 names, assembled as a Blob so large exports need not sit in memory.
// Plain zip limits apply: under 65535 entries and 4 GB.

const CRC_TABLE = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c;
});

function crc32(data) {
  let crc = 0xffffffff;
  for (let i = 0; i < data.length; i++) crc = CRC_TABLE[(crc ^ data[i]) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

const encoder = new TextEncoder();
const LIMIT = 0xffffffff;

export class Zip {
  #parts = [];
  #central = [];
  #offset = 0;
  #time;
  #date;

  constructor(now = new Date()) {
    this.#time = (now.getHours() << 11) | (now.getMinutes() << 5) | (now.getSeconds() >> 1);
    this.#date = ((now.getFullYear() - 1980) << 9) | ((now.getMonth() + 1) << 5) | now.getDate();
  }

  add(path, data) {
    if (this.#central.length === 0xffff) throw new Error("too many files for a zip; export to a folder instead");
    if (this.#offset + data.length + 1024 > LIMIT) throw new Error("over 4 GB, too large for a zip; export to a folder instead");
    const name = encoder.encode(path);
    const crc = crc32(data);
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    this.#common(local, 4, crc, data.length);
    local.setUint16(26, name.length, true);
    this.#parts.push(local, name, new Blob([data]));

    const central = new DataView(new ArrayBuffer(46));
    central.setUint32(0, 0x02014b50, true);
    central.setUint16(4, 20, true); // made by: version 2.0
    this.#common(central, 6, crc, data.length);
    central.setUint16(28, name.length, true);
    central.setUint32(42, this.#offset, true);
    this.#central.push(central, name);
    this.#offset += 30 + name.length + data.length;
  }

  // The fields local and central headers share, from "version needed" to the sizes.
  #common(view, at, crc, size) {
    view.setUint16(at, 20, true); // version needed: 2.0
    view.setUint16(at + 2, 0x0800, true); // names are UTF-8
    view.setUint16(at + 4, 0, true); // stored
    view.setUint16(at + 6, this.#time, true);
    view.setUint16(at + 8, this.#date, true);
    view.setUint32(at + 10, crc, true);
    view.setUint32(at + 14, size, true);
    view.setUint32(at + 18, size, true);
  }

  finish() {
    const size = this.#central.reduce((sum, part) => sum + part.byteLength, 0);
    const end = new DataView(new ArrayBuffer(22));
    end.setUint32(0, 0x06054b50, true);
    end.setUint16(8, this.#central.length / 2, true);
    end.setUint16(10, this.#central.length / 2, true);
    end.setUint32(12, size, true);
    end.setUint32(16, this.#offset, true);
    return new Blob([...this.#parts, ...this.#central, end], { type: "application/zip" });
  }
}
