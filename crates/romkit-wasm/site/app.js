// The page: hands the dropped ROM to the worker, then browses, previews, and exports what it finds.

const $ = (id) => document.getElementById(id);
const SHOWN = 500;
const ARCHIVES = new Set(["narc", "garc"]);

// Requests to the worker, answered in order through their id.
const worker = new Worker(new URL("worker.js", import.meta.url), { type: "module" });
const pending = new Map();
let nextId = 0;
worker.onmessage = ({ data: { id, ok, error, progress } }) => {
  const request = pending.get(id);
  if (progress) return request.onProgress?.(progress);
  pending.delete(id);
  error === undefined ? request.resolve(ok) : request.reject(new Error(error));
};
function ask(type, args = {}, onProgress) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject, onProgress });
    worker.postMessage({ id, type, ...args });
  });
}

let rom = null; // {name, overview, assets, files}
let tab = "assets";
let archive = null; // {path, members} while browsing an archive's members
let previewUrl = null;

// Opening.
const drop = $("drop");
$("pick").addEventListener("change", (event) => event.target.files[0] && open(event.target.files[0]));
drop.addEventListener("dragover", (event) => {
  event.preventDefault();
  drop.classList.add("over");
});
drop.addEventListener("dragleave", () => drop.classList.remove("over"));
drop.addEventListener("drop", (event) => {
  event.preventDefault();
  drop.classList.remove("over");
  const file = event.dataTransfer.files[0];
  if (file) open(file);
});

async function open(file) {
  status(`Reading ${file.name}…`);
  $("rom").hidden = true;
  try {
    const opened = await ask("open", { file });
    rom = { name: file.name.replace(/\.[^.]+$/, ""), ...opened };
  } catch (error) {
    return status(`${file.name}: ${error.message}`, true);
  }
  archive = null;
  showOverview();
  tab = rom.assets ? "assets" : "files";
  $("exports").hidden = !rom.assets;
  status(rom.overview.supported ? "" : "romkit recognizes this ROM but cannot extract its data yet; its files can still be browsed.");
  $("rom").hidden = false;
  selectTab(tab);
}

function showOverview() {
  const { identity, game_name, banner } = rom.overview;
  const rows = [
    ["Game", game_name ?? "not in the catalog"],
    ["Platform", platformName(identity.platform)],
    ["Header title", identity.title],
    ["Product code", identity.code ?? "none"],
    ["Region", identity.region],
    ["Revision", identity.revision],
  ];
  if (banner) rows.push(["Banner", banner.english.replaceAll("\n", " · ")]);
  if (rom.assets) rows.push(["Assets", rom.assets.length.toLocaleString()]);
  if (rom.files) rows.push(["Files", rom.files.length.toLocaleString()]);
  $("overview").replaceChildren(
    ...rows.flatMap(([term, value]) => [el("dt", term), el("dd", String(value))]),
  );
}

function platformName(platform) {
  return (
    { game_boy: "Game Boy", game_boy_color: "Game Boy Color", game_boy_advance: "Game Boy Advance", nintendo_ds: "Nintendo DS", nintendo3ds: "Nintendo 3DS" }[platform] ??
    platform
  );
}

// Browsing.
document.querySelectorAll(".tabs button").forEach((button) =>
  button.addEventListener("click", () => selectTab(button.dataset.tab)),
);
$("filter").addEventListener("input", render);

function selectTab(name) {
  const browsable = Boolean(rom.assets || rom.files);
  document.querySelector(".tabs").hidden = !browsable;
  document.querySelector(".browser").hidden = !browsable;
  if (!browsable) return;
  tab = name;
  archive = null;
  document.querySelectorAll(".tabs button").forEach((button) => {
    const available = button.dataset.tab === "assets" ? rom.assets : rom.files;
    button.hidden = !available;
    button.setAttribute("aria-selected", String(button.dataset.tab === name));
  });
  $("filter").value = "";
  $("preview").replaceChildren(el("p", "Select an entry to preview it."));
  $("preview").firstChild.className = "muted";
  render();
}

// Entries of the current view: [{path, label, size?, archive?}].
function entries() {
  if (tab === "assets") return rom.assets.map((path) => ({ path, label: path }));
  if (archive) {
    return [
      { up: true, label: `↩ back to the files (in ${archive.path})` },
      ...archive.members.map((member) => ({ path: archive.path, member: Number(member.path), label: `${archive.path} #${member.path}`, size: member.size, format: member.format })),
    ];
  }
  return rom.files.map((file) => ({ path: file.path, label: file.path, size: file.size, format: file.format }));
}

function render() {
  const words = $("filter").value.toLowerCase().split(/\s+/).filter(Boolean);
  const all = entries();
  const matches = all.filter((entry) => entry.up || words.every((word) => entry.label.toLowerCase().includes(word)));
  const count = matches.filter((entry) => !entry.up).length;
  $("count").textContent =
    count > SHOWN ? `${count.toLocaleString()} matches, showing the first ${SHOWN}; filter to narrow them down.` : `${count.toLocaleString()} entries`;
  $("entries").replaceChildren(
    ...matches.slice(0, SHOWN + 1).map((entry) => {
      const item = el("li");
      item.append(el("span", entry.label));
      const note = [entry.format?.extension, entry.size !== undefined && formatSize(entry.size)].filter(Boolean).join(" · ");
      if (note) item.append(el("small", note));
      item.addEventListener("click", () => {
        document.querySelector("#entries li.selected")?.classList.remove("selected");
        item.classList.add("selected");
        choose(entry);
      });
      return item;
    }),
  );
}

async function choose(entry) {
  if (entry.up) {
    archive = null;
    return render();
  }
  if (tab === "files" && !archive && ARCHIVES.has(entry.format?.extension)) {
    try {
      archive = { path: entry.path, members: await ask("archive", { path: entry.path }) };
      $("filter").value = "";
      return render();
    } catch {
      // Not readable as an archive after all: preview it as a plain file.
    }
  }
  const name = entry.member === undefined ? entry.path : `${entry.path}.${entry.member}`;
  preview(name, () =>
    tab === "assets"
      ? ask("extract", { path: entry.path })
      : entry.member === undefined
        ? ask("read", { path: entry.path })
        : ask("readMember", { path: entry.path, member: entry.member }),
  );
}

async function preview(name, load) {
  const pane = $("preview");
  pane.replaceChildren(el("h2", name), el("p", "Loading…"));
  let data;
  try {
    data = await load();
  } catch (error) {
    return pane.replaceChildren(el("h2", name), el("p", error.message));
  }
  if (previewUrl) URL.revokeObjectURL(previewUrl);
  const type = mimeType(name);
  previewUrl = URL.createObjectURL(new Blob([data], { type }));
  const body = [];
  if (type.startsWith("image/")) {
    const image = el("img");
    image.src = previewUrl;
    image.alt = name;
    body.push(image);
  } else if (type.startsWith("audio/")) {
    const audio = el("audio");
    audio.controls = true;
    audio.src = previewUrl;
    body.push(audio);
  } else if (type === "application/json" || type.startsWith("text/")) {
    const text = new TextDecoder().decode(data.subarray(0, 200_000));
    body.push(el("pre", data.length > 200_000 ? `${text}\n… (truncated; download for the rest)` : text));
  } else {
    const dump = el("pre", hexdump(data.subarray(0, 512)));
    dump.className = "hex";
    body.push(dump);
  }
  const download = el("a", `Download (${formatSize(data.length)})`);
  download.href = previewUrl;
  download.download = name.split("/").pop();
  pane.replaceChildren(el("h2", name), ...body, el("p", "", download));
}

function mimeType(name) {
  const extension = name.split(".").pop().toLowerCase();
  return (
    { png: "image/png", wav: "audio/wav", json: "application/json", txt: "text/plain", mid: "audio/midi", glb: "model/gltf-binary" }[extension] ??
    "application/octet-stream"
  );
}

function hexdump(bytes) {
  const lines = [];
  for (let at = 0; at < bytes.length; at += 16) {
    const row = [...bytes.subarray(at, at + 16)];
    const hex = row.map((byte) => byte.toString(16).padStart(2, "0")).join(" ");
    const text = row.map((byte) => (byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : ".")).join("");
    lines.push(`${at.toString(16).padStart(6, "0")}  ${hex.padEnd(47)}  ${text}`);
  }
  return lines.join("\n");
}

// Exporting.
const folderButton = $("export-folder");
folderButton.hidden = !("showDirectoryPicker" in window);
folderButton.addEventListener("click", async () => {
  let dir;
  try {
    dir = await window.showDirectoryPicker({ id: "romkit-export", mode: "readwrite" });
  } catch {
    return; // cancelled
  }
  await exporting(async (onProgress) => {
    const count = await ask("exportFolder", { dir }, onProgress);
    status(`Exported ${count.toLocaleString()} files to the folder “${dir.name}”.`);
  });
});

$("export-zip").addEventListener("click", () =>
  exporting(async (onProgress) => {
    const zip = await ask("exportZip", { name: `${rom.name}.zip` }, onProgress);
    const link = el("a");
    link.href = URL.createObjectURL(zip);
    link.download = zip.name;
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 60_000);
    status(`Saved ${zip.name} (${formatSize(zip.size)}).`);
  }),
);

async function exporting(run) {
  const buttons = document.querySelectorAll("#exports button");
  const bar = $("progress");
  buttons.forEach((button) => (button.disabled = true));
  bar.hidden = false;
  bar.removeAttribute("value");
  try {
    await run(([done, total]) => {
      bar.max = total;
      bar.value = done;
      status(`Exporting… ${done.toLocaleString()} of ${total.toLocaleString()} assets`);
    });
  } catch (error) {
    status(`Export failed: ${error.message}`, true);
  } finally {
    buttons.forEach((button) => (button.disabled = false));
    bar.hidden = true;
  }
}

// Helpers.
function status(text, error = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", error);
}

function el(tag, text, ...children) {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  node.append(...children);
  return node;
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

fetch("version.txt")
  .then((response) => (response.ok ? response.text() : ""))
  .then((version) => ($("version").textContent = version.trim() && `romkit ${version.trim()}`));
