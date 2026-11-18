// Generate tests/fixtures/anilist-corpus.json and refresh the node payloads of
// tests/fixtures/anilist-relations.json from the live AniList API.
//
//   node tests/fixtures/generate.mjs
//
// The corpus was first recorded from live search caches; this script
// refetches every recorded query (the
// normalized key is exactly what the Rust matcher searches) so new AniList
// fields — `season`, `status`, `genres` — reach the fixtures too. The suite
// never touches the network; regenerate only when the query set, the fetched
// fields or the scorer's normalization change.
//
// `anilist-relations.json`'s franchise chain structure stays hand-mapped from
// `docs/research/season-resolution.md`; only each node's payload is refetched
// here (same fields, by id), so the absolute-episode walk still runs offline.
//
// The API allows ~30 requests/minute: every call waits 2.15 s, and a 429
// waits out its Retry-After instead of failing the run. A full corpus pass
// takes ~5 minutes.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ENDPOINT = "https://graphql.anilist.co";
const THROTTLE_MS = 2150;

// The same field set the Rust queries fetch (src/anilist.rs), minus
// `coverImage`: the fixtures trim it, the Candidate tolerates it missing.
const FIELDS =
  "id format seasonYear season status episodes genres " +
  "title { romaji english } synonyms";

const SEARCH_QUERY = `query ($search: String) {
Page (perPage: 10) { media (search: $search, type: ANIME, sort: SEARCH_MATCH) {
${FIELDS} } } }`;

const ENTRY_QUERY = `query ($id: Int) {
Media (id: $id) { ${FIELDS} } }`;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function post(query, variables) {
  for (;;) {
    const response = await fetch(ENDPOINT, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ query, variables }),
    });
    if (response.status === 429) {
      const after = Number(response.headers.get("Retry-After")) * 1000;
      const ms = Number.isFinite(after) && after > 0 ? after : 60_000;
      console.log(`429: waiting ${ms / 1000} s`);
      await sleep(ms);
      continue;
    }
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} for ${JSON.stringify(variables)}`);
    }
    return (await response.json()).data;
  }
}

async function fetchAll(query, variablesList) {
  const results = [];
  for (const variables of variablesList) {
    results.push(await post(query, variables));
    await sleep(THROTTLE_MS);
  }
  return results;
}

function norm(s) {
  return s
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

// The corpus fixture shape: every field the Candidate reads, no extras.
function trim(media) {
  return {
    id: media.id,
    format: media.format,
    seasonYear: media.seasonYear,
    season: media.season,
    status: media.status,
    episodes: media.episodes,
    genres: media.genres ?? [],
    title: {
      romaji: media.title?.romaji ?? null,
      english: media.title?.english ?? null,
    },
    synonyms: media.synonyms ?? [],
  };
}

// --- corpus: refetch every recorded query -------------------------------

const corpusPath = join(HERE, "anilist-corpus.json");
const corpus = JSON.parse(readFileSync(corpusPath, "utf8"));
const keys = Object.keys(corpus);
if (process.argv[2] !== "relations") {
  console.log(`corpus: refetching ${keys.length} queries`);

  const searches = await fetchAll(
    SEARCH_QUERY,
    keys.map((key) => ({ search: key })),
  );
  for (const [index, data] of searches.entries()) {
    const key = keys[index];
    const media = data?.Page?.media ?? [];
    if (media.length === 0) console.error(`empty result: ${key}`);
    corpus[key] = media.map(trim);
  }
  writeFileSync(corpusPath, JSON.stringify(corpus, null, 1) + "\n");
  console.log(`wrote anilist-corpus.json: ${keys.length} queries`);
}

// --- relations: enrich each node payload by id --------------------------

// The file's keys are Root Season ids; each value is the SEQUEL chain's
// candidate nodes, hand-mapped in `docs/research/season-resolution.md`.
// Only the payloads are refetched — the chain structure stays hand-edited.
const relationsPath = join(HERE, "anilist-relations.json");
const relations = JSON.parse(readFileSync(relationsPath, "utf8"));
const nodeIds = new Map(); // id -> [path, index] occurrences
for (const [rootId, nodes] of Object.entries(relations)) {
  if (!Array.isArray(nodes)) {
    throw new Error(`chain for root ${rootId} is not an array`);
  }
  nodes.forEach((node, index) => nodeIds.set(node.id, [rootId, index]));
}
console.log(`relations: refetching ${nodeIds.size} nodes`);

const entries = await fetchAll(
  ENTRY_QUERY,
  [...nodeIds.keys()].map((id) => ({ id })),
);
for (const [index, data] of entries.entries()) {
  const media = data?.Media;
  const id = [...nodeIds.keys()][index];
  if (!media) {
    console.error(`no entry: ${id}`);
    continue;
  }
  const [rootId, position] = nodeIds.get(id);
  relations[rootId][position] = trim(media);
}
writeFileSync(relationsPath, JSON.stringify(relations, null, 1) + "\n");
console.log(`wrote anilist-relations.json: ${nodeIds.size} nodes`);
