// Writes THIRD_PARTY_LICENSES.md: everything the released app is built from that isn't this
// project's own, with each licence's text. Most of those licences (MIT, Apache-2.0, BSD)
// ask that their notice travel with the binary, so the file is bundled into the app
// (tauri.conf.json, bundle.resources) and `scripts/release.sh` rewrites it for each release.
//
//   bun scripts/licenses.js          # or: just licenses
//
// Rust crates come from `cargo metadata`, walked from the app over what a Mac build links
// (not build scripts, not tests); the page's packages from app/node_modules. What neither
// can see (libraries inside sherpa-onnx's prebuilt binaries, the voice models the app
// downloads) is listed by hand in NATIVE.
import { readdirSync, readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";

const root = dirname(import.meta.dir);
const OUT = join(root, "THIRD_PARTY_LICENSES.md");
const TEXTS = /^(licen[sc]e|copying|notice|unlicense|copyright)/i;

/** The texts of the licence files in a package's folder. */
function texts(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => TEXTS.test(name))
    .sort()
    .map((name) => {
      try {
        return readFileSync(join(dir, name), "utf8").replace(/\r/g, "").trim();
      } catch {
        return ""; // a folder named LICENSES
      }
    })
    .filter(Boolean);
}

function crates() {
  const found = new Map();
  for (const target of ["aarch64-apple-darwin", "x86_64-apple-darwin"]) {
    const run = Bun.spawnSync(["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target], {
      cwd: root,
      stderr: "inherit",
    });
    if (run.exitCode !== 0) throw new Error("cargo metadata failed");
    const meta = JSON.parse(run.stdout.toString());
    const packages = new Map(meta.packages.map((p) => [p.id, p]));
    const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
    const ours = new Set(meta.workspace_members);
    const app = meta.packages.find((p) => p.name === "tokidoki-app").id;
    const seen = new Set([app]);
    const queue = [app];
    while (queue.length) {
      for (const dep of nodes.get(queue.pop()).deps) {
        // Only what is linked into the app: `kind: null` is an ordinary dependency.
        if (seen.has(dep.pkg) || !dep.dep_kinds.some((k) => k.kind === null)) continue;
        seen.add(dep.pkg);
        queue.push(dep.pkg);
      }
    }
    for (const id of seen) {
      if (ours.has(id)) continue;
      const p = packages.get(id);
      found.set(`${p.name} ${p.version}`, {
        name: p.name,
        version: p.version,
        license: p.license ?? "see its text",
        home: p.repository ?? p.homepage ?? "",
        texts: texts(dirname(p.manifest_path)),
      });
    }
  }
  return [...found.values()];
}

function packages() {
  const modules = join(root, "app/node_modules");
  const read = (name) => JSON.parse(readFileSync(join(modules, name, "package.json"), "utf8"));
  // What ends up in the page: the app's dependencies, Svelte's runtime, and what they need.
  const queue = [...Object.keys(read("..").dependencies), "svelte"];
  const found = new Map();
  while (queue.length) {
    const name = queue.pop();
    if (found.has(name) || !existsSync(join(modules, name, "package.json"))) continue;
    const p = read(name);
    const home = typeof p.repository === "string" ? p.repository : (p.repository?.url ?? p.homepage ?? "");
    found.set(name, {
      name,
      version: p.version,
      license: typeof p.license === "string" ? p.license : "see its text",
      home: home.replace(/^git\+/, "").replace(/\.git$/, ""),
      texts: texts(join(modules, name)),
    });
    queue.push(...Object.keys(p.dependencies ?? {}));
  }
  return [...found.values()];
}

// Not visible to cargo or bun. sherpa-onnx's prebuilt static libraries are linked into the
// app whole; the models are downloaded by the app on first use of voice, not shipped in it.
const NATIVE = [
  ["sherpa-onnx", "Apache-2.0", "https://github.com/k2-fsa/sherpa-onnx", "speech recognition and synthesis; linked into the app"],
  ["ONNX Runtime", "MIT", "https://github.com/microsoft/onnxruntime", "runs the models; linked into the app"],
  ["espeak-ng", "GPL-3.0-or-later", "https://github.com/espeak-ng/espeak-ng", "turns text into phonemes for speaking; linked into the app"],
  ["piper-phonemize", "MIT", "https://github.com/rhasspy/piper-phonemize", "linked into the app"],
  ["kaldi-native-fbank, kaldi-decoder, kaldifst, OpenFst", "Apache-2.0", "https://github.com/k2-fsa", "linked into the app"],
  ["SentencePiece (simple-sentencepiece)", "Apache-2.0", "https://github.com/pkufool/simple-sentencepiece", "linked into the app"],
  ["Whisper large-v3-turbo", "MIT", "https://github.com/openai/whisper", "the listening model; downloaded on first use"],
  ["Kokoro v1.0", "Apache-2.0", "https://huggingface.co/hexgrad/Kokoro-82M", "the speaking model; downloaded on first use"],
];

const byName = (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version);
const rust = crates().sort(byName);
const page = packages().sort(byName);

// Many crates carry the same text (Apache-2.0 word for word): each text is printed once,
// under the names of everything that ships it.
const groups = new Map();
const missing = [];
for (const item of [...rust, ...page]) {
  if (!item.texts.length) missing.push(item);
  for (const text of item.texts) {
    if (!groups.has(text)) groups.set(text, []);
    groups.get(text).push(item);
  }
}

const row = (i) => `| ${i.name} | ${i.version} | ${i.license} | ${i.home} |`;
const table = (items) => ["| Name | Version | Licence | Source |", "| --- | --- | --- | --- |", ...items.map(row)].join("\n");
const out = [
  "# Third-party licences",
  "",
  "Tokidoki is MIT licensed (`LICENSE`). The released app is also built from the work listed",
  "here, each under its own licence. This file is written by `scripts/licenses.js`; don't",
  "edit it by hand.",
  "",
  "## Native libraries and models",
  "",
  "| Name | Licence | Source | How it is used |",
  "| --- | --- | --- | --- |",
  ...NATIVE.map((n) => `| ${n.join(" | ")} |`),
  "",
  `## Rust crates (${rust.length})`,
  "",
  table(rust),
  "",
  `## Packages in the app's page (${page.length})`,
  "",
  table(page),
  "",
  "## Licence texts",
  "",
];
for (const [text, items] of [...groups].sort((a, b) => byName(a[1][0], b[1][0]))) {
  out.push(`### ${items.map((i) => `${i.name} ${i.version}`).join(", ")}`, "", "```text", text.replaceAll("```", "'''"), "```", "");
}
if (missing.length) {
  out.push(
    "### Published without a licence file",
    "",
    "These name their licence (in the tables above) and ship no text of it:",
    "",
    missing.map((i) => `${i.name} ${i.version} (${i.license})`).join(", ") + ".",
    "",
  );
}
await Bun.write(OUT, out.join("\n"));
console.log(`${OUT}: ${rust.length} crates, ${page.length} packages, ${groups.size} texts, ${missing.length} without a text`);
