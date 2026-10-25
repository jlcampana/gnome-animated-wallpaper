import Soup from "gi://Soup";
import GLib from "gi://GLib";
import GdkPixbuf from "gi://GdkPixbuf";

// Covers smaller than this (browsers hand over ~120px thumbnails) get
// replaced by the store artwork when the track can be matched.
export const SMALL_COVER_PX = 300;

const HD_SIZE = 1000;
const TIMEOUT_S = 6;

const MAX_QUERIES = 6;
const MIN_SCORE = 0.62;
const GOOD_SCORE = 0.85;

const decode = (data) => new TextDecoder().decode(data);

const norm = (s) =>
  (s || "")
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[\s\p{P}\p{S}]+/gu, "");

// Bracketed tags and "feat." credits that video titles add to the song name
const cleanTitle = (t) =>
  t
    .replace(/[(\[【（][^)\]】）]*[)\]】）]/g, " ")
    .replace(/\s-[^-]+-\s*$/, "")
    .replace(/\s(?:feat|ft|prod)\.?\s.*$/i, "")
    .replace(/\bm\/v\b/gi, " ")
    .replace(/\b(?:official|music|lyrics?|video|audio|mv|hd|4k)\b/gi, " ")
    .replace(/\s+/g, " ")
    .trim();

const cleanArtist = (a) =>
  a
    .replace(/\s*-\s*topic$/i, "")
    .replace(/vevo$/i, "")
    .replace(/\s+official$/i, "")
    .trim();

// "이효리 (Lee Hyori)" → both names
function aliases(name) {
  const m = name.match(/^(.*?)\s*[(（](.+?)[)）]\s*$/);
  return m ? [m[1].trim(), m[2].trim()].filter(Boolean) : [name.trim()];
}

// Store region that is likely to list the track, from the script it uses
function localStore(text) {
  if (/[\u3040-\u30ff\u4e00-\u9fff]/.test(text)) return "JP";
  return null;
}

function trackVariants(artist, title) {
  const base = title
    .replace(/[[【][^\]】]*[\]】]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const pairs = [[artist, title]];

  // Quoted song names: LiSA 『紅蓮華』, NewJeans (뉴진스) 'Super Shy'
  const quoted = base.match(/^(.*?)[「『'‘"“](.+?)[」』'’"”]/);
  if (quoted)
    pairs.push([quoted[1].replace(/[\s\-–—:]+$/, "") || artist, quoted[2]]);

  // "Artist - Song" (or "Song / Artist") packed into the title
  const parts = base.split(/\s+[-–—/|]\s+/);
  if (parts.length === 2) {
    pairs.push([parts[0], parts[1]]);
    pairs.push([parts[1], parts[0]]);
  }

  const out = [];
  const seen = new Set();
  for (const [a, t] of pairs) {
    const songTitle = cleanTitle(t) || t;
    for (const name of aliases(cleanArtist(a))) {
      const key = norm(name) + "|" + norm(songTitle);
      if (!norm(name) || !norm(songTitle) || seen.has(key)) continue;
      seen.add(key);
      out.push({ artist: name, title: songTitle });
    }
  }
  return out.length ? out : [{ artist, title }];
}

function bigrams(s) {
  const out = new Map();
  for (let i = 0; i < s.length - 1; i++) {
    const g = s.slice(i, i + 2);
    out.set(g, (out.get(g) || 0) + 1);
  }
  return out;
}

/** Similarity in [0, 1]; containment counts as a strong match. */
function similarity(x, y) {
  const a = norm(x);
  const b = norm(y);
  if (!a || !b) return 0;
  if (a === b) return 1;
  if (a.includes(b) || b.includes(a))
    return 0.8 + 0.2 * (Math.min(a.length, b.length) / Math.max(a.length, b.length));
  if (a.length < 2 || b.length < 2) return 0;

  const ga = bigrams(a);
  const gb = bigrams(b);
  let common = 0;
  for (const [g, n] of ga) common += Math.min(n, gb.get(g) || 0);
  return (2 * common) / (a.length - 1 + (b.length - 1));
}

// Latin words of a mixed-script title: "Hoodie에 반바지" → "Hoodie"
const latinPart = (t) =>
  t.replace(/[^\p{Script=Latin}\d\s]+/gu, " ").replace(/\s+/g, " ").trim();

function scoreResult(r, v, album) {
  const trackName = cleanTitle(r.trackName || "");
  const artist = similarity(r.artistName, v.artist);
  let title = similarity(trackName, v.title);

  // Stores often romanise Korean titles; trust the Latin words only when
  // the artist is clearly the same.
  const latin = latinPart(v.title);
  if (artist >= 0.8 && latin !== v.title && norm(latin).length >= 4)
    title = Math.max(title, 0.85 * similarity(trackName, latin));

  if (title < 0.5) return 0;
  let score = 0.6 * title + 0.4 * artist;
  if (album) score += 0.15 * similarity(r.collectionName, album);
  return score;
}

/** @returns {boolean} true when the local cover is too small to look sharp */
export function isSmallCover(uri) {
  try {
    const path = uri.startsWith("file://")
      ? GLib.filename_from_uri(uri)[0]
      : uri;
    const [format, w, h] = GdkPixbuf.Pixbuf.get_file_info(path);
    return !!format && Math.max(w, h) < SMALL_COVER_PX;
  } catch {
    return false;
  }
}

export class HdCoverFinder {
  constructor() {
    this._session = new Soup.Session();
    this._session.timeout = TIMEOUT_S;
    this._misses = new Set();
    this._inflight = new Map();
    this._dir = GLib.build_filenamev([
      GLib.get_user_cache_dir(),
      "mpris-covers",
      "hd",
    ]);
    GLib.mkdir_with_parents(this._dir, 0o755);
  }

  /**
   * Looks the track up in the iTunes catalogue and caches its artwork.
   * @returns {Promise<string|null>} file:// URI of the HD cover, or null
   */
  find({ title, artists, album }) {
    const artist = (artists || []).join(" ");
    if (!title || !artist) return Promise.resolve(null);

    const key = GLib.compute_checksum_for_string(
      GLib.ChecksumType.SHA1,
      `${norm(artist)}|${norm(title)}|${norm(album)}`,
      -1,
    );
    const path = GLib.build_filenamev([this._dir, `${key}.jpg`]);
    if (GLib.file_test(path, GLib.FileTest.EXISTS))
      return Promise.resolve(`file://${path}`);
    if (this._misses.has(key)) return Promise.resolve(null);

    let job = this._inflight.get(key);
    if (!job) {
      job = this._lookup(artist, title, album, path)
        .catch(() => null)
        .then((uri) => {
          this._inflight.delete(key);
          if (!uri) this._misses.add(key);
          return uri;
        });
      this._inflight.set(key, job);
    }
    return job;
  }

  async _lookup(artist, title, album, path) {
    const hit = await this._bestMatch(artist, title, album);
    if (!hit?.artworkUrl100) return null;

    const art = hit.artworkUrl100.replace(
      /\/\d+x\d+bb\./,
      `/${HD_SIZE}x${HD_SIZE}bb.`,
    );
    GLib.file_set_contents(path, await this._get(art));
    return `file://${path}`;
  }

  // Tries a few query variants and keeps the best-scoring result, so
  // "Artist - Song (Official Video)" from a "Artist - Topic" channel matches.
  async _bestMatch(artist, title, album) {
    const variants = trackVariants(artist, title);
    const local = localStore(`${artist} ${title}`);
    const queries = [];
    for (const v of variants) {
      queries.push([`${v.artist} ${v.title}`, "US"]);
      if (local) queries.push([`${v.artist} ${v.title}`, local]);
    }
    queries.push([variants[variants.length - 1].title, local ?? "US"]);

    let best = null;
    const seen = new Set();
    for (const [term, country] of queries.slice(0, MAX_QUERIES)) {
      if (seen.has(term + country)) continue;
      seen.add(term + country);

      const url =
        `https://itunes.apple.com/search?entity=song&limit=15` +
        `&country=${country}&term=${encodeURIComponent(term)}`;
      let results;
      try {
        results = JSON.parse(decode(await this._get(url))).results || [];
      } catch {
        continue;
      }

      for (const r of results) {
        const score = Math.max(
          ...variants.map((v) => scoreResult(r, v, album)),
        );
        if (!best || score > best.score) best = { r, score };
      }
      if (best?.score >= GOOD_SCORE) break;
    }
    return best?.score >= MIN_SCORE ? best.r : null;
  }

  async _get(url) {
    const msg = Soup.Message.new("GET", url);
    const bytes = await this._session.send_and_read_async(
      msg,
      GLib.PRIORITY_LOW,
      null,
    );
    if (msg.status_code !== 200) throw new Error(`HTTP ${msg.status_code}`);
    const raw = bytes.get_data();
    if (!raw?.length) throw new Error("empty body");
    return raw;
  }

  destroy() {
    this._session.abort();
    this._inflight.clear();
  }
}
