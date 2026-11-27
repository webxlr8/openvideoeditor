#!/usr/bin/env node
// Builds the desktop-extension package (.mcpb) that installs the editor's MCP server in a client that supports
// desktop extensions (add-agent-skills-and-connectors D8). The package is a zip with a manifest and the bundled `ove`
// command line, which it starts as `ove mcp`: the "binary" server type, so the client needs no other runtime.
//
//   node tools/mcpb/pack.mjs --platform win32|darwin --version X.Y.Z --server <folder> --out <file.mcpb>
//   node tools/mcpb/pack.mjs --platform win32|darwin --version X.Y.Z --manifest <file.json>   (manifest only)
//   node tools/mcpb/pack.mjs --self-test
//
// <folder> holds what goes under `server/` in the package:
//   win32:  ove.exe and the FFmpeg DLLs next to it, plus licenses/
//   darwin: MacOS/ove and Frameworks/*.dylib (the app's layout, so ove's @executable_path/../Frameworks rpath holds),
//           plus licenses/
// Each package carries the binary of one platform and lists only that platform. No dependency: Node's zlib only.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";

const ENTRY = { win32: "server/ove.exe", darwin: "server/MacOS/ove" };

export function manifest(platform, version) {
  const entry = ENTRY[platform];
  if (!entry) throw new Error(`platform must be win32 or darwin, not ${platform}`);
  return {
    manifest_version: "0.3",
    name: "openvideoeditor",
    display_name: "OpenVideoEditor",
    version,
    description: "Edit OpenVideoEditor video projects: read and change the timeline, add media, check frames, export.",
    long_description:
      "Runs the editor's own command line (`ove mcp`) as a local MCP server. Edits to a project open in the app appear there at once and can be undone. Skills installed in the editor are offered as prompts. Nothing is sent anywhere by the server itself.",
    author: { name: "OpenVideoEditor" },
    license: "Apache-2.0",
    server: {
      type: "binary",
      entry_point: entry,
      mcp_config: {
        command: `\${__dirname}/${entry}`,
        args: ["mcp"],
        env: { OVE_LIBRARY: "${user_config.library}" },
      },
    },
    user_config: {
      library: {
        type: "directory",
        title: "Library folder",
        description: "The folder that holds your projects. Leave empty to use the app's library.",
        required: false,
        default: "",
      },
    },
    tools_generated: true,
    prompts_generated: true,
    keywords: ["video", "editing", "timeline"],
    compatibility: { platforms: [platform] },
  };
}

// ---------------------------------------------------------------------------------------------------------------
// Zip (deflate), with Unix modes so the server binary stays executable.

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** entries: [{ name, data: Buffer, mode }] → zip Buffer. */
export function zip(entries) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const e of entries) {
    const name = Buffer.from(e.name, "utf8");
    const raw = e.data;
    const packed = zlib.deflateRawSync(raw, { level: 9 });
    const crc = crc32(raw);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0x0800, 6); // UTF-8 names
    local.writeUInt16LE(8, 8); // deflate
    local.writeUInt32LE(0, 10); // time, date
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28);
    locals.push(local, name, packed);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE((3 << 8) | 20, 4); // made by Unix, version 2.0
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x0800, 8);
    central.writeUInt16LE(8, 10);
    central.writeUInt32LE(0, 12);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(packed.length, 20);
    central.writeUInt32LE(raw.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt16LE(0, 30);
    central.writeUInt16LE(0, 32);
    central.writeUInt16LE(0, 34);
    central.writeUInt16LE(0, 36);
    central.writeUInt32LE(((0o100000 | (e.mode ?? 0o644)) << 16) >>> 0, 38);
    central.writeUInt32LE(offset, 42);
    centrals.push(central, name);
    offset += local.length + name.length + packed.length;
  }
  const dir = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(dir.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, dir, end]);
}

/** Reads a zip written by `zip` back: [{ name, data, mode }]. */
export function unzip(buf) {
  const end = buf.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  const count = buf.readUInt16LE(end + 10);
  let p = buf.readUInt32LE(end + 16);
  const out = [];
  for (let i = 0; i < count; i++) {
    const size = buf.readUInt32LE(p + 20);
    const nameLen = buf.readUInt16LE(p + 28);
    const extra = buf.readUInt16LE(p + 30) + buf.readUInt16LE(p + 32);
    const mode = buf.readUInt32LE(p + 38) >>> 16;
    const crc = buf.readUInt32LE(p + 16);
    const local = buf.readUInt32LE(p + 42);
    const name = buf.toString("utf8", p + 46, p + 46 + nameLen);
    const lName = buf.readUInt16LE(local + 26);
    const lExtra = buf.readUInt16LE(local + 28);
    const start = local + 30 + lName + lExtra;
    const data = zlib.inflateRawSync(buf.subarray(start, start + size));
    if (crc32(data) !== crc) throw new Error(`${name}: CRC mismatch`);
    out.push({ name, data, mode: mode & 0o777 });
    p += 46 + nameLen + extra;
  }
  return out;
}

function walk(dir, base = dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((d) => {
    const full = path.join(dir, d.name);
    if (d.isSymbolicLink()) throw new Error(`${full} is a link; package real files only`);
    return d.isDirectory() ? walk(full, base) : [path.relative(base, full).split(path.sep).join("/")];
  });
}

export function pack(platform, version, serverDir) {
  const m = manifest(platform, version);
  const files = walk(serverDir).sort();
  const entry = m.server.entry_point.slice("server/".length);
  if (!files.includes(entry)) throw new Error(`${serverDir} has no ${entry}`);
  if (!files.some((f) => f.startsWith("licenses/")))
    throw new Error(`${serverDir} has no licenses/ folder (the FFmpeg LGPL notices must ship with the libraries)`);
  const entries = [{ name: "manifest.json", data: Buffer.from(JSON.stringify(m, null, 2) + "\n"), mode: 0o644 }];
  for (const f of files) {
    const exec = f === entry || /\.(dylib|dll|so)$/.test(f);
    entries.push({ name: `server/${f}`, data: fs.readFileSync(path.join(serverDir, f)), mode: exec ? 0o755 : 0o644 });
  }
  return zip(entries);
}

function selfTest() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mcpb-"));
  try {
    for (const [platform, entry] of Object.entries(ENTRY)) {
      const server = path.join(dir, platform);
      const bin = path.join(server, entry.slice("server/".length));
      fs.mkdirSync(path.dirname(bin), { recursive: true });
      fs.writeFileSync(bin, Buffer.alloc(70_000, 7));
      fs.mkdirSync(path.join(server, "licenses"));
      fs.writeFileSync(path.join(server, "licenses", "NOTICE.md"), "notices");
      const back = unzip(pack(platform, "1.2.3", server));
      const names = back.map((e) => e.name).sort();
      const want = ["manifest.json", "server/licenses/NOTICE.md", entry].sort();
      if (JSON.stringify(names) !== JSON.stringify(want)) throw new Error(`${platform}: ${names}`);
      const m = JSON.parse(back[0].data.toString());
      if (m.server.type !== "binary" || m.server.mcp_config.args[0] !== "mcp") throw new Error("server");
      if (m.compatibility.platforms.join() !== platform) throw new Error("platforms");
      const bin2 = back.find((e) => e.name === entry);
      if (bin2.mode !== 0o755 || bin2.data.length !== 70_000) throw new Error("binary mode or size");
    }
    let refused = false;
    try {
      fs.rmSync(path.join(dir, "win32", "licenses"), { recursive: true });
      pack("win32", "1.2.3", path.join(dir, "win32"));
    } catch {
      refused = true;
    }
    if (!refused) throw new Error("a package without licences must be refused");
    console.log("mcpb self-test passed");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : undefined;
}

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith("pack.mjs")) {
  if (process.argv.includes("--self-test")) {
    selfTest();
  } else {
    const platform = arg("platform");
    const version = arg("version");
    if (!platform || !version) {
      console.error("usage: pack.mjs --platform win32|darwin --version X.Y.Z (--server <dir> --out <file> | --manifest <file>)");
      process.exit(2);
    }
    if (arg("manifest")) {
      fs.writeFileSync(arg("manifest"), JSON.stringify(manifest(platform, version), null, 2) + "\n");
    } else {
      const out = arg("out");
      fs.writeFileSync(out, pack(platform, version, arg("server")));
      console.log(`wrote ${out} (${fs.statSync(out).size} bytes)`);
    }
  }
}
