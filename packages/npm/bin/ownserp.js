#!/usr/bin/env node
"use strict";

const { createHash } = require("node:crypto");
const { execFileSync, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const VERSION = "1.0.0";
const DIGEST = "49e558d05df20d08cd2f9fcb0b3cd4779e732e471e66afd76ee8b5ce1c902471";
const ASSET = `ownserp-${VERSION}-x86_64-unknown-linux-gnu.tar.gz`;
const BASE = `https://github.com/devops-infinity/ownserp/releases/download/v${VERSION}`;
const ALLOWED_HOSTS = new Set([
  "github.com",
  "objects.githubusercontent.com",
  "release-assets.githubusercontent.com",
]);
const REQUEST_TIMEOUT_MS = 30000;
const MAX_ARCHIVE_BYTES = 64 * 1024 * 1024;
const TAR = fs.existsSync("/bin/tar") ? "/bin/tar" : "tar";
const CACHE_ROOT = path.join(
  process.env.OWN_SERPER_CACHE_DIR ||
    path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"), "ownserp"),
);
const CACHE = path.join(CACHE_ROOT, VERSION);
const BINARY = path.join(CACHE, "ownserp");
const STAMP = path.join(CACHE, "digest");

function allowed(url) {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" && ALLOWED_HOSTS.has(parsed.hostname);
  } catch {
    return false;
  }
}

function download(url, destination, redirects = 0) {
  return new Promise((resolve, reject) => {
    if (redirects > 5) {
      reject(new Error("too many redirects"));
      return;
    }
    if (!allowed(url)) {
      reject(new Error(`refusing to download from ${url}`));
      return;
    }
    const request = https.get(
      url,
      { headers: { "User-Agent": `ownserp-npm/${VERSION}` }, timeout: REQUEST_TIMEOUT_MS },
      (response) => {
        const status = response.statusCode || 0;
        if (status >= 300 && status < 400 && response.headers.location) {
          response.resume();
          download(response.headers.location, destination, redirects + 1).then(
            resolve,
            reject,
          );
          return;
        }
        if (status !== 200) {
          response.resume();
          reject(new Error(`${url} answered ${status}`));
          return;
        }
        const announced = Number(response.headers["content-length"] || 0);
        if (announced > MAX_ARCHIVE_BYTES) {
          response.resume();
          reject(new Error(`${url} announced ${announced} bytes, above the limit`));
          return;
        }
        const file = fs.createWriteStream(destination);
        let written = 0;
        let stopped = false;
        response.on("data", (chunk) => {
          written += chunk.length;
          if (!stopped && written > MAX_ARCHIVE_BYTES) {
            stopped = true;
            response.destroy();
            file.destroy();
            fs.rmSync(destination, { force: true });
            reject(new Error(`${url} passed the ${MAX_ARCHIVE_BYTES} byte limit`));
          }
        });
        response.on("error", reject);
        response.pipe(file);
        file.on("finish", () => file.close(resolve));
        file.on("error", reject);
      },
    );
    request.on("timeout", () => request.destroy(new Error("download timed out")));
    request.on("error", reject);
  });
}

async function fetchArchive(destination) {
  let lastError;
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      await download(`${BASE}/${ASSET}`, destination);
      return;
    } catch (error) {
      lastError = error;
      fs.rmSync(destination, { force: true });
    }
  }
  throw lastError;
}

function safeLocal(target, directory) {
  if (process.platform === "win32" || !process.getuid) {
    return true;
  }
  try {
    const stat = fs.lstatSync(target);
    if (directory ? !stat.isDirectory() : !stat.isFile()) {
      return false;
    }
    return stat.uid === process.getuid() && (stat.mode & 0o022) === 0;
  } catch {
    return false;
  }
}

function verified() {
  try {
    if (!fs.existsSync(BINARY) || !safeLocal(BINARY, false) || !safeLocal(STAMP, false)) {
      return false;
    }
    const stamped = fs.readFileSync(STAMP, "utf8").trim();
    if (stamped.length !== 64) {
      return false;
    }
    const actual = createHash("sha256").update(fs.readFileSync(BINARY)).digest("hex");
    return actual === stamped;
  } catch {
    return false;
  }
}

function pruneOldVersions() {
  let entries;
  try {
    entries = fs.readdirSync(CACHE_ROOT, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (
      !entry.isDirectory() ||
      entry.name === VERSION ||
      !/^\d+\.\d+\.\d+$/.test(entry.name)
    ) {
      continue;
    }
    const directory = path.join(CACHE_ROOT, entry.name);
    if (
      fs.existsSync(path.join(directory, "ownserp")) ||
      fs.existsSync(path.join(directory, "digest"))
    ) {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  }
}

async function ensureBinary() {
  if (DIGEST.length !== 64) {
    throw new Error("this package was published without a pinned checksum; install the release archive instead");
  }
  if (verified()) {
    return;
  }
  fs.mkdirSync(CACHE, { recursive: true, mode: 0o700 });
  if (!safeLocal(CACHE, true)) {
    throw new Error(
      `the cache directory ${CACHE} is writable by another user; remove it or set OWN_SERPER_CACHE_DIR to a private path`,
    );
  }
  const archive = path.join(CACHE, ASSET);
  await fetchArchive(archive);
  const actual = createHash("sha256").update(fs.readFileSync(archive)).digest("hex");
  if (actual !== DIGEST) {
    fs.rmSync(archive, { force: true });
    throw new Error("the download did not match the checksum pinned in this package");
  }
  const staging = fs.mkdtempSync(path.join(CACHE, ".extract-"));
  try {
    execFileSync(TAR, ["-xzf", archive, "-C", staging, "--no-same-owner"]);
    const unpacked = path.join(staging, "ownserp");
    if (!fs.existsSync(unpacked)) {
      throw new Error("the archive did not contain the ownserp binary");
    }
    fs.chmodSync(unpacked, 0o755);
    const installed = createHash("sha256").update(fs.readFileSync(unpacked)).digest("hex");
    fs.rmSync(BINARY, { force: true });
    fs.renameSync(unpacked, BINARY);
    fs.writeFileSync(STAMP, `${installed}\n`);
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
    fs.rmSync(archive, { force: true });
  }
  pruneOldVersions();
}

async function main() {
  try {
    await ensureBinary();
  } catch (error) {
    console.error(`ownserp: could not prepare the binary: ${error.message}`);
    process.exit(1);
  }
  const result = spawnSync(BINARY, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) {
    console.error(`ownserp: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status === null ? 1 : result.status);
}

main();
