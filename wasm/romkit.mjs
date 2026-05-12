// A minimal host for romkit-wasm, usable in a Web Worker or in Node.
//
// `readAt(offset, length)` must synchronously return the ROM's bytes at that range as a
// Uint8Array: in a worker, `new Uint8Array(new FileReaderSync().readAsArrayBuffer(
// file.slice(offset, offset + length)))`; in Node, `fs.readSync` into a buffer.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export async function load(wasm, readAt) {
  let exports;
  let files = [];
  let result;
  const view = (ptr, len) => new Uint8Array(exports.memory.buffer, ptr, len);

  ({ instance: { exports } } = await WebAssembly.instantiate(wasm, {
    romkit: {
      read_at(offset, dst, len) {
        try {
          view(dst, len).set(readAt(offset, len));
          return 0;
        } catch {
          return 1;
        }
      },
      emit_file(path, pathLen, data, dataLen) {
        files.push({ path: decoder.decode(view(path, pathLen)), data: view(data, dataLen).slice() });
        return 0;
      },
      emit_result(json, len) {
        result = JSON.parse(decoder.decode(view(json, len)));
      },
    },
  }));

  // Call an export, passing strings through romkit's allocator; returns the result and files.
  function call(name, ...args) {
    const allocated = [];
    const raw = args.flatMap((arg) => {
      if (typeof arg !== "string") return [arg];
      const bytes = encoder.encode(arg);
      const ptr = exports.romkit_alloc(bytes.length);
      view(ptr, bytes.length).set(bytes);
      allocated.push([ptr, bytes.length]);
      return [ptr, bytes.length];
    });
    files = [];
    try {
      exports[name](...raw);
    } finally {
      for (const [ptr, len] of allocated) exports.romkit_free(ptr, len);
    }
    if ("error" in result) throw new Error(result.error);
    return { value: result.ok, files };
  }

  return {
    /** Open a ROM of `size` bytes; returns its overview. */
    open: (size) => call("romkit_open", size).value,
    close: () => exports.romkit_close(),
    /** The ROM's files: [{path, size, format}]. */
    files: () => call("romkit_files").value,
    /** The members of the archive at `path`. */
    archive: (path) => call("romkit_archive", path).value,
    /** A file's bytes, or one archive member's. */
    read: (path, member = -1) => call("romkit_read", path, member).files[0].data,
    /** Paths of the assets romkit can extract. */
    assets: () => call("romkit_assets").value,
    /** One asset's bytes. */
    extract: (path) => call("romkit_extract", path).files[0].data,
    /** Every asset plus the manifest: {value: manifest, files: [{path, data}]}. */
    export: () => call("romkit_export"),
  };
}
