import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { captionSource, captionsForMedia, subtitlePolicy, TheaterSubtitles, SUBTITLE_PREFERENCE_KEY } from "./theater-subtitles.ts";

test("top-right subtitle controls stay hidden without removing caption bindings", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  assert.match(html, /<section\b[^>]*id="subtitle-controls"[^>]*\bhidden\s*>/);
  for (const id of ["theater-subtitle-hud", "theater-subtitles", "theater-subtitle-explanation"]) {
    assert.ok(html.includes(`id="${id}"`));
  }
  assert.match(css, /\[hidden\]\s*\{\s*display:\s*none\s*!important;/);
});

test("subtitle font is reduced by 35 percent across the responsive range", () => {
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
  const rule = css.match(/\.theater-subtitle-hud\s*\{([^}]+)\}/)?.[1];
  const sizes = rule?.match(/font:\s*600 clamp\(([\d.]+)rem, ([\d.]+)vw, ([\d.]+)rem\)/);
  assert.ok(sizes);
  for (const [index, original] of [1.25, 2.8, 2].entries()) {
    assert.ok(Math.abs(Number(sizes[index + 1]) - original * 0.65) < 1e-10);
  }
});

test("subtitle policy is independent of audibility", () => {
  assert.deepEqual(subtitlePolicy(false, false), { enabled: false, forced: false, panel: 1 });
  assert.deepEqual(subtitlePolicy(true, false), { enabled: true, forced: false, panel: 3 });
  assert.deepEqual(subtitlePolicy(false, true), { enabled: false, forced: false, panel: 1 });
  assert.deepEqual(subtitlePolicy(true, true), { enabled: true, forced: false, panel: 3 });
});
test("captions are optional and only attach to their same-origin movie", () => {
  const entry = { source: "/movie.mp4", captions: "/movie.vtt" };
  assert.equal(captionSource(entry, "https://example.com/movie.mp4", "https://example.com"), "https://example.com/movie.vtt");
  for (const captions of [undefined, "https://evil.test/a.vtt", "//evil.test/a.vtt", "/a.mp4", "data:text/vtt,a"]) {
    assert.equal(captionSource({ ...entry, captions }, entry.source, "https://example.com"), undefined);
  }
  assert.equal(captionSource(entry, "/other.mp4", "https://example.com"), undefined);
});
test("caption association follows the actual movie after manual pause/seek or a schedule boundary", () => {
  const origin = "https://example.com";
  const first = { source: "/movie.mp4", captions: "/first.vtt" };
  const scheduled = { source: "/movie.mp4", captions: "/scheduled.vtt" };
  const next = { source: "/next.mp4", captions: "/next.vtt" };
  const entries = [first, scheduled, next];
  const actual = origin + first.source;
  assert.equal(captionsForMedia(entries, scheduled, actual, origin), origin + scheduled.captions);
  assert.equal(captionsForMedia(entries, next, actual, origin), origin + first.captions);
  assert.equal(captionsForMedia(entries, undefined, actual, origin), origin + first.captions);
  assert.equal(captionsForMedia(entries, scheduled, "/unknown.mp4", origin), undefined);
  assert.equal(captionsForMedia(entries, scheduled, "", origin), undefined);
  // A matching scheduled entry without captions must not borrow another slot's captions.
  assert.equal(captionsForMedia(entries, { source: first.source }, actual, origin), undefined);

  const f = fixture(null);
  f.control.setSource(captionsForMedia(entries, scheduled, f.media.source, origin));
  const track = f.tracks[0];
  f.load(track, "Movie cue");
  f.control.update(false); // Paused audio does not change preference or association.
  f.control.setSource(captionsForMedia(entries, scheduled, f.media.source, origin));
  assert.equal(f.tracks.length, 1);
  assert.equal(f.hud.textContent, "Movie cue");
  f.media.currentTime = 3;
  f.media.dispatchEvent(new Event("seeked"));
  assert.equal(f.hud.hidden, true, "cues follow video time, not UTC schedule time");
  assert.equal(track.removed, undefined);
  f.control.dispose();
});

function fixture(saved) {
  const tracks = [], writes = [], panels = [];
  const media = Object.assign(new EventTarget(), { source: "/movie.mp4", currentTime: 1, seeking: false,
    getAttribute() { return this.source; }, append(track) { tracks.push(track); },
    ownerDocument: { createElement() {
      return Object.assign(new EventTarget(), { readyState: 0, track: Object.assign(new EventTarget(), { mode: "disabled", activeCues: [] }), remove() { this.removed = true; } });
    } },
  });
  const button = Object.assign(new EventTarget(), { setAttribute(key, value) { this[key] = value; } });
  const hud = { textContent: "", hidden: true }, explanation = {};
  const storage = { getItem() { if (saved instanceof Error) throw saved; return saved; }, setItem(key, value) { writes.push([key, value]); } };
  const control = new TheaterSubtitles({ media, button, hud, explanation, storage, panel: (state) => panels.push(state) });
  const load = (track, text) => {
    track.readyState = 2;
    track.track.activeCues = [{ startTime: 0, endTime: 2, getCueAsHTML: () => ({ textContent: text }) }];
    track.dispatchEvent(new Event("load"));
  };
  return { media, button, hud, explanation, control, tracks, writes, panels, load };
}
test("saved off is honored while inaudible; toggles persist independently and storage defaults on", () => {
  const f = fixture("off");
  assert.equal(f.button.disabled, false);
  assert.equal(f.control.enabled, false);
  assert.equal(f.panels.at(-1), 1);
  f.control.toggle(); assert.deepEqual(f.writes, [[SUBTITLE_PREFERENCE_KEY, "on"]]);
  assert.equal(f.control.enabled, true);
  f.control.update(false); assert.equal(f.panels.at(-1), 3);
  f.control.update(true); assert.equal(f.panels.at(-1), 3);
  f.button.dispatchEvent(new Event("click"));
  f.control.update(false);
  assert.equal(f.control.enabled, false);
  assert.equal(f.panels.at(-1), 1);
  assert.deepEqual(f.writes.at(-1), [SUBTITLE_PREFERENCE_KEY, "off"]);
  for (const saved of [null, new Error("blocked")]) {
    const other = fixture(saved); assert.equal(other.control.enabled, true); assert.equal(other.panels.at(-1), 3); other.control.dispose();
  }
  f.control.dispose();
});
test("inaudible captions can be hidden and restored without changing the movie or track", () => {
  const f = fixture("off");
  f.control.setSource("/movie.vtt"); const track = f.tracks[0];
  f.load(track, "Current cue");
  assert.equal(f.hud.hidden, true);
  f.control.toggle();
  assert.equal(f.hud.textContent, "Current cue");
  f.control.toggle(); f.control.update(false);
  assert.equal(f.hud.hidden, true);
  assert.equal(f.hud.textContent, "");
  assert.equal(f.tracks.length, 1);
  assert.equal(f.media.source, "/movie.mp4");
  assert.equal(f.media.currentTime, 1);
  f.control.dispose();
});

test("native hidden tracks render plain text, suppress stale loads/cues and clean up without touching video", () => {
  const f = fixture(null);
  f.control.setSource("/one.vtt"); const old = f.tracks[0];
  assert.equal(old.track.mode, "hidden");
  f.load(old, "<script>literal text</script>\nSecond line");
  assert.equal(f.hud.textContent, "<script>literal text</script>\nSecond line");
  f.media.seeking = true; f.media.dispatchEvent(new Event("seeking")); assert.equal(f.hud.hidden, true);
  f.media.seeking = false; f.media.dispatchEvent(new Event("seeked")); assert.equal(f.hud.hidden, false);
  f.media.source = "/next.mp4"; f.control.update(false);
  assert.equal(old.removed, true); assert.equal(old.track.mode, "disabled"); assert.equal(f.hud.hidden, true);
  f.control.setSource("/two.vtt"); const next = f.tracks[1];
  f.load(old, "stale"); assert.equal(f.hud.textContent, "");
  next.dispatchEvent(new Event("error")); assert.equal(f.hud.hidden, true);
  f.load(next, "Current"); assert.equal(f.hud.textContent, "Current");
  f.media.currentTime = 3; next.track.dispatchEvent(new Event("cuechange")); assert.equal(f.hud.hidden, true);
  f.control.dispose(); f.load(next, "late");
  assert.equal(f.hud.hidden, true); assert.equal(next.removed, true); assert.equal(f.media.source, "/next.mp4");
});
