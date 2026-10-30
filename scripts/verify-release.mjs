import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';

const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url)));
const config = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url)));
const cargo = readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const lock = readFileSync(new URL('../src-tauri/Cargo.lock', import.meta.url), 'utf8');
const notes = readFileSync(new URL('../RELEASE_NOTES.md', import.meta.url), 'utf8');
const version = pkg.version;
assert.match(version, /^\d+\.\d+\.\d+$/);
assert.equal(config.version, version, 'Tauri version must match package.json');
assert.equal(cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1], version, 'Cargo version mismatch');
assert.equal(lock.match(/\[\[package\]\]\r?\nname = "ndm"\r?\nversion = "([^"]+)"/)?.[1], version, 'Cargo.lock version mismatch');
assert.ok(notes.includes(`v${version}`), 'Release notes must describe this version');
if (process.env.RELEASE_TAG) assert.equal(process.env.RELEASE_TAG, `v${version}`, 'Release tag mismatch');
console.log(`Release versions verified: v${version}`);
