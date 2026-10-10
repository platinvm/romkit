// Runs romkit off the main thread. The ROM is read in place from its File, a slice at a time,
// through FileReaderSync: opening a 4 GB 3DS image reads a few dozen kilobytes.
//
// Requests are {id, type, ...}; every one gets a {id, ok} or {id, error} reply, and exports
// also post {id, progress: [done, total]} along the way.

import { load } from "./romkit.mjs";
import { Zip } from "./zip.js";

const reader = new FileReaderSync();
let file = null;
const romkit = fetch(new URL("romkit.wasm", import.meta.url))
  .then((response) => {
    if (!response.ok) throw new Error(`romkit.wasm: HTTP ${response.status}`);
    return response.arrayBuffer();
  })
  .then((wasm) =>
    load(wasm, (offset, length) =>
      new Uint8Array(reader.readAsArrayBuffer(file.slice(offset, offset + length))),
    ),
  );

const handlers = {
  async open({ file: opened }) {
    const rom = await romkit;
    file = opened;
    const overview = rom.open(file.size);
    // Game Boy and GBA ROMs have no filesystem.
    const files = attempt(() => rom.files());
    return {
      overview,
      assets: overview.supported ? rom.assets() : null,
      files: files?.length ? files : null,
    };
  },

  async extract({ path }) {
    return (await romkit).extract(path);
  },

  async read({ path }) {
    return (await romkit).read(path);
  },

  async archive({ path }) {
    return (await romkit).archive(path);
  },

  async readMember({ path, member }) {
    return (await romkit).read(path, member);
  },

  // Into a zip assembled from Blobs, which browsers may keep on disk rather than in memory.
  async exportZip({ name }, progress) {
    const rom = await romkit;
    const zip = new Zip();
    const manifest = rom.manifest();
    await eachAsset(rom, manifest.value.assets, progress, (path, data) => zip.add(path, data));
    zip.add("manifest.json", manifest.files[0].data);
    return new File([zip.finish()], name, { type: "application/zip" });
  },

  // Straight into a folder the user picked (File System Access API, Chromium browsers).
  async exportFolder({ dir }, progress) {
    const rom = await romkit;
    const manifest = rom.manifest();
    const folders = new Map([["", dir]]);
    const write = async (path, data) => {
      const slash = path.lastIndexOf("/");
      const folder = await subfolder(folders, path.slice(0, Math.max(slash, 0)));
      const handle = await folder.getFileHandle(path.slice(slash + 1), { create: true });
      const writable = await handle.createWritable();
      await writable.write(data);
      await writable.close();
    };
    await eachAsset(rom, manifest.value.assets, progress, write);
    await write("manifest.json", manifest.files[0].data);
    return manifest.value.assets.length + 1;
  },
};

async function eachAsset(rom, assets, progress, sink) {
  let last = 0;
  for (let i = 0; i < assets.length; i++) {
    await sink(assets[i], rom.extract(assets[i]));
    if (Date.now() - last > 150) {
      progress([i + 1, assets.length]);
      last = Date.now();
    }
  }
  progress([assets.length, assets.length]);
}

async function subfolder(folders, path) {
  if (!folders.has(path)) {
    const slash = path.lastIndexOf("/");
    const parent = await subfolder(folders, path.slice(0, Math.max(slash, 0)));
    folders.set(path, await parent.getDirectoryHandle(path.slice(slash + 1), { create: true }));
  }
  return folders.get(path);
}

function attempt(f) {
  try {
    return f();
  } catch {
    return null;
  }
}

// One request at a time: romkit holds a single open ROM and is not reentrant.
let queue = Promise.resolve();
self.onmessage = ({ data: { id, type, ...args } }) => {
  queue = queue.then(async () => {
    try {
      const ok = await handlers[type](args, (progress) => self.postMessage({ id, progress }));
      self.postMessage({ id, ok });
    } catch (error) {
      self.postMessage({ id, error: String(error?.message ?? error) });
    }
  });
};
