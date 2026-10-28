import test from "node:test";
import assert from "node:assert/strict";
import { setupSniffer } from "./sniffer.js";

function event() {
  const listeners = [];
  return { addListener: listener => listeners.push(listener), emit: (...args) => listeners.forEach(listener => listener(...args)) };
}
const settle = async () => { for (let index = 0; index < 12; index++) await new Promise(resolve => setImmediate(resolve)); };

test("network sniffing stays per tab, clears navigation, and forwards only known candidates", async () => {
  const session = {};
  const tabs = new Map([[1, { id: 1, url: "https://page.test/watch", incognito: false }], [2, { id: 2, url: "https://other.test/", incognito: false }], [3, { id: 3, url: "https://private.test/", incognito: true }]]);
  globalThis.chrome = {
    tabs: { get: async id => tabs.get(id), onRemoved: event() },
    webRequest: { onBeforeRequest: event(), onHeadersReceived: event(), onCompleted: event(), onErrorOccurred: event() },
    storage: { onChanged: event(), session: {
      get: async key => structuredClone({ [key]: session[key] }),
      set: async data => Object.assign(session, structuredClone(data)),
      remove: async key => { delete session[key]; },
    } },
  };
  let config = { videoSniff: true, excludeSites: "" };
  const sent = [];
  const options = { getConfig: async () => config, sendToNdm: async capture => sent.push(capture), runInPage: async () => [] };
  const handle = setupSniffer(options);
  const begin = (requestId, tabId, type = "xmlhttprequest") => chrome.webRequest.onBeforeRequest.emit({ requestId, tabId, type });
  const receive = (requestId, tabId, url, mime, statusCode = 200) => chrome.webRequest.onHeadersReceived.emit({ requestId, tabId, url, statusCode, responseHeaders: [{ name: "Content-Type", value: mime }] });
  begin("manifest", 1);
  receive("manifest", 1, "https://cdn.test/play?token=x", "application/x-mpegurl");
  begin("movie", 2);
  receive("movie", 2, "https://cdn.test/movie.mp4", "video/mp4", 206);
  begin("chunk", 1);
  receive("chunk", 1, "https://cdn.test/a.m4s", "video/mp4");
  await settle();
  const first = await handle({ type: "media-list", tabId: 1 });
  assert.equal(first.length, 1);
  assert.equal(first[0].hls, true);
  assert.equal((await handle({ type: "media-list", tabId: 2 })).length, 1);
  assert.deepEqual(await handle({ type: "media-download", tabId: 1, url: first[0].url }), { ok: true });
  assert.deepEqual(sent[0], { url: first[0].url, hls: true, referrer: tabs.get(1).url });
  assert.ok((await handle({ type: "media-download", tabId: 1, url: "https://evil.test/new.m3u8" })).error);
  begin("stale", 1);
  begin("navigation", 1, "main_frame");
  receive("stale", 1, "https://cdn.test/stale.m3u8", "application/x-mpegurl");
  await settle();
  assert.deepEqual(await handle({ type: "media-list", tabId: 1 }), []);
  begin("private", 3);
  receive("private", 3, "https://cdn.test/secret.m3u8", "application/x-mpegurl");
  await settle();
  assert.deepEqual(await handle({ type: "media-list", tabId: 3 }), []);
  assert.equal(session["media:3"], undefined);
  config = { videoSniff: false, excludeSites: "" };
  chrome.storage.onChanged.emit({ videoSniff: { newValue: false } }, "sync");
  await settle();
  assert.equal(session["media:2"], undefined);
  assert.deepEqual(await handle({ type: "media-list", tabId: 2 }), []);
  config = { videoSniff: true, excludeSites: "other.test" };
  assert.deepEqual(await handle({ type: "media-list", tabId: 2 }), []);
  chrome.tabs.onRemoved.emit(2);
  await settle();
  assert.equal(session["media:2"], undefined);
  delete globalThis.chrome;
});
