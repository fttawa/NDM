// node extension/rules.test.mjs
import assert from "node:assert/strict";
import { DEFAULTS, extOf, hostMatches, shouldIntercept } from "./rules.js";

const cfg = (o = {}) => ({ ...DEFAULTS, ...o });
const item = (o = {}) => ({ url: "https://dl.example.com/a/file.zip", filename: "file.zip", totalBytes: 5 << 20, ...o });

assert.equal(extOf("C:\\x\\a.tar.gz"), "gz");
assert.equal(extOf("/path/noext"), "");
assert.ok(hostMatches("cdn.github.com", "github.com"));
assert.ok(!hostMatches("notgithub.com", "github.com"));

assert.ok(shouldIntercept(item(), cfg()).ok);
assert.ok(!shouldIntercept(item(), cfg({ enabled: false })).ok);
assert.ok(!shouldIntercept(item({ url: "blob:https://x/1" }), cfg()).ok, "blob 不拦截");
assert.ok(!shouldIntercept(item({ incognito: true }), cfg()).ok, "无痕不拦截");
assert.ok(!shouldIntercept(item({ byExtensionId: "me" }), cfg(), "me").ok, "自身发起不拦截");
assert.ok(!shouldIntercept(item({ totalBytes: 100 }), cfg({ minSizeKB: 1024 })).ok, "太小不拦截");
assert.ok(shouldIntercept(item({ totalBytes: -1, fileSize: 0 }), cfg({ minSizeKB: 1024 })).ok, "大小未知时拦截");
assert.ok(!shouldIntercept(item(), cfg({ excludeSites: "example.com" })).ok, "子域名排除");
assert.ok(shouldIntercept(item(), cfg({ types: "exe, .zip" })).ok, "类型匹配");
assert.ok(!shouldIntercept(item({ filename: "a.pdf", url: "https://x.com/a.pdf" }), cfg({ types: "zip exe" })).ok, "类型不匹配");
assert.ok(shouldIntercept(item({ filename: "", url: "https://x.com/d/setup.EXE?x=1" }), cfg({ types: "exe" })).ok, "从 URL 取扩展名");
console.log("rules: 全部通过");
