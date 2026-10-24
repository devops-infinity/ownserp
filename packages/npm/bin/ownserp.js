#!/usr/bin/env node
"use strict";

const { createHash } = require("node:crypto");
const { execFileSync, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const VERSION = "0.1.1";
const ASSET = `ownserp-${VERSION}-x86_64-unknown-linux-gnu.tar.gz`;
const BASE = `https://github.com/devops-infinity/ownserp/releases/download/v${VERSION}`;
const CACHE = path.join(
  process.env.OWN_SERPER_CACHE_DIR ||
    path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"), "ownserp"),
  VERSION,
);
const BINARY = path.join(CACHE, "ownserp");

function download(url, destination, redirects = 0) {
  return new Promise((resolve, reject) => {
    if (redirects > 5) {
      reject(new Error("too many redirects"));
      return;
    }
    https
      .get(url, { headers: { "User-Agent": `ownserp-npm/${VERSION}` } }, (response) => {
        const status = response.statusCode || 0;
        if (status >= 300 && status < 400 && response.headers.location) {
          response.resume();
          download(response.headers.location, destination, redirects + 1).then(resolve, reject);
          return;
        }
        if (status !== 200) {
          response.resume();
          reject(new Error(`${url} answered ${status}`));
          return;
        }
        const file = fs.createWriteStream(destination);
        response.pipe(file);
        file.on("finish", () => file.close(resolve));
        file.on("error", reject);
      })
      .on("error", reject);
  });
}

async function ensureBinary() {
  if (fs.existsSync(BINARY)) {
    return;
  }
  fs.mkdirSync(CACHE, { recursive: true });
  const archive = path.join(CACHE, ASSET);
  const checksum = `${archive}.sha256`;
  await download(`${BASE}/${ASSET}`, archive);
  await download(`${BASE}/${ASSET}.sha256`, checksum);
  const expected = fs.readFileSync(checksum, "utf8").trim().split(/\s+/)[0];
  const actual = createHash("sha256").update(fs.readFileSync(archive)).digest("hex");
  if (expected !== actual) {
    fs.rmSync(archive, { force: true });
    fs.rmSync(checksum, { force: true });
    throw new Error("the download did not match its published checksum");
  }
  execFileSync("tar", ["-xzf", archive, "-C", CACHE]);
  fs.chmodSync(BINARY, 0o755);
  fs.rmSync(archive, { force: true });
  fs.rmSync(checksum, { force: true });
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
