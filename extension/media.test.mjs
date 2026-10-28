import test from "node:test";
import assert from "node:assert/strict";
import { mediaCandidate, mergeMedia } from "./media.js";

test("detects HLS by URL or MIME and excludes non-media and fragments", () => {
  assert.equal(mediaCandidate("https://cdn.test/index.M3U8?token=x").hls, true);
  assert.equal(mediaCandidate("https://cdn.test/play?id=1", "application/vnd.apple.mpegurl; charset=UTF-8").hls, true);
  assert.equal(mediaCandidate("https://cdn.test/movie", "video/mp4").hls, false);
  assert.equal(mediaCandidate("https://cdn.test/video.webm").hls, false);
  for (const url of ["blob:https://page.test/123", "file:///movie.mp4", "https://user:pass@cdn.test/video.mp4", "https://cdn.test/a.m4s", "https://cdn.test/a.ts", "https://cdn.test/a.zip"]) {
    assert.equal(mediaCandidate(url), null);
  }
  assert.equal(mediaCandidate("https://cdn.test/a.m4s", "video/mp4"), null);
});

test("deduplicates exact signed URLs, upgrades HLS, and bounds the list", () => {
  let items = [];
  const video = mediaCandidate("https://cdn.test/play?token=1", "video/mp4");
  items = mergeMedia(items, video);
  items = mergeMedia(items, { ...video, hls: true });
  assert.equal(items.length, 1);
  assert.equal(items[0].hls, true);
  items = mergeMedia(items, { ...video, url: "https://cdn.test/play?token=2" });
  assert.equal(items.length, 2);
  for (let index = 0; index < 150; index++) items = mergeMedia(items, { ...video, url: `https://cdn.test/${index}.mp4` });
  assert.equal(items.length, 100);
  assert.ok(items[0].url.endsWith("50.mp4"));
});
