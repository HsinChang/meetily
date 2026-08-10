#!/usr/bin/env node
// fetch-bundled-models.mjs
//
// Downloads the models that are shipped built-in with the app so the first-run
// "download a model" step can be skipped. Files land in `src-tauri/models/`,
// mirroring the on-disk layout the app expects under its app-data `models/` dir:
//
//   models/ggml-large-v3-turbo.bin
//   models/Qwen3.5-4B-Q4_K_M.gguf
//   models/parakeet/parakeet-tdt-0.6b-v3-int8/{encoder-model.int8.onnx,
//       decoder_joint-model.int8.onnx, nemo128.onnx, vocab.txt}
//   models/funasr/fun-asr-nano-2512-q8/{funasr-encoder-f16.gguf, qwen3-0.6b-q8_0.gguf}
//
// This tree is registered as a Tauri resource (see tauri.conf.json
// `bundle.resources`) and copied into the app-data models dir on first launch
// by `models_seed.rs`.
//
// URLs mirror the ones already encoded in the Rust engines:
//   - whisper_engine/whisper_engine.rs (HF ggerganov/whisper.cpp)
//   - summary/summary_engine/models.rs (HF unsloth Qwen3.5)
//   - parakeet_engine/parakeet_engine.rs (meetily v3 mirror)
//   - config.rs `funasr_model_files` (HF FunAudioLLM/Fun-ASR-Nano-GGUF)
//
// Usage:  node scripts/fetch-bundled-models.mjs
// Existing files with a plausible size are skipped, so re-runs are cheap.

import { createWriteStream } from 'node:fs';
import { mkdir, stat, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';

const __dirname = dirname(fileURLToPath(import.meta.url));
const MODELS_DIR = resolve(__dirname, '..', 'models');
const PARAKEET_DIR = join(MODELS_DIR, 'parakeet', 'parakeet-tdt-0.6b-v3-int8');
const FUNASR_DIR = join(MODELS_DIR, 'funasr', 'fun-asr-nano-2512-q8');

const WHISPER_BASE = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main';
const QWEN_BASE = 'https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main';
const PARAKEET_BASE =
  'https://meetily.towardsgeneralintelligence.com/models/parakeet-tdt-0.6b-v3-onnx';
const FUNASR_BASE = 'https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-GGUF/resolve/main';

// dest (relative to MODELS_DIR) -> { url, minBytes } (minBytes guards partial files)
const FILES = [
  {
    dest: 'ggml-large-v3-turbo.bin',
    url: `${WHISPER_BASE}/ggml-large-v3-turbo.bin`,
    minBytes: 1_400_000_000, // ~1.55 GB
  },
  {
    dest: 'Qwen3.5-4B-Q4_K_M.gguf',
    url: `${QWEN_BASE}/Qwen3.5-4B-Q4_K_M.gguf`,
    minBytes: 2_000_000_000, // ~2.5 GB
  },
  {
    dest: 'parakeet/parakeet-tdt-0.6b-v3-int8/encoder-model.int8.onnx',
    url: `${PARAKEET_BASE}/encoder-model.int8.onnx`,
    minBytes: 580_000_000,
  },
  {
    dest: 'parakeet/parakeet-tdt-0.6b-v3-int8/decoder_joint-model.int8.onnx',
    url: `${PARAKEET_BASE}/decoder_joint-model.int8.onnx`,
    minBytes: 8_000_000,
  },
  {
    dest: 'parakeet/parakeet-tdt-0.6b-v3-int8/nemo128.onnx',
    url: `${PARAKEET_BASE}/nemo128.onnx`,
    minBytes: 100_000,
  },
  {
    dest: 'parakeet/parakeet-tdt-0.6b-v3-int8/vocab.txt',
    url: `${PARAKEET_BASE}/vocab.txt`,
    minBytes: 5_000,
  },
  // Fun-ASR-Nano: Chinese-first ASR. minBytes must stay >= the thresholds in
  // config.rs `funasr_model_files`, or the app treats a seeded file as truncated.
  {
    dest: 'funasr/fun-asr-nano-2512-q8/funasr-encoder-f16.gguf',
    url: `${FUNASR_BASE}/funasr-encoder-f16.gguf`,
    minBytes: 460_000_000, // ~469 MB
  },
  {
    dest: 'funasr/fun-asr-nano-2512-q8/qwen3-0.6b-q8_0.gguf',
    url: `${FUNASR_BASE}/qwen3-0.6b-q8_0.gguf`,
    minBytes: 790_000_000, // ~805 MB
  },
];

function fmtBytes(n) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)} KB`;
  return `${n} B`;
}

async function fileSize(path) {
  try {
    return (await stat(path)).size;
  } catch {
    return -1;
  }
}

async function download({ dest, url, minBytes }) {
  const outPath = join(MODELS_DIR, dest);
  const existing = await fileSize(outPath);
  if (existing >= minBytes) {
    console.log(`✓ ${dest} already present (${fmtBytes(existing)}), skipping`);
    return;
  }
  if (existing >= 0) {
    console.log(`… ${dest} exists but looks incomplete (${fmtBytes(existing)}), re-downloading`);
    await rm(outPath, { force: true });
  }

  await mkdir(dirname(outPath), { recursive: true });
  console.log(`↓ ${dest}  <-  ${url}`);

  const res = await fetch(url, { redirect: 'follow' });
  if (!res.ok || !res.body) {
    throw new Error(`Failed to download ${url}: HTTP ${res.status} ${res.statusText}`);
  }

  const total = Number(res.headers.get('content-length')) || 0;
  let received = 0;
  let lastLog = Date.now();
  const body = Readable.fromWeb(res.body);
  body.on('data', (chunk) => {
    received += chunk.length;
    const now = Date.now();
    if (now - lastLog > 1000) {
      lastLog = now;
      const pct = total ? ` (${((received / total) * 100).toFixed(1)}%)` : '';
      process.stdout.write(`\r    ${fmtBytes(received)}${total ? ' / ' + fmtBytes(total) : ''}${pct}   `);
    }
  });

  const tmpPath = `${outPath}.part`;
  await pipeline(body, createWriteStream(tmpPath));
  process.stdout.write('\n');

  const got = await fileSize(tmpPath);
  if (got < minBytes) {
    await rm(tmpPath, { force: true });
    throw new Error(`${dest} downloaded ${fmtBytes(got)} but expected at least ${fmtBytes(minBytes)}`);
  }
  // Atomic-ish rename into place
  const { rename } = await import('node:fs/promises');
  await rename(tmpPath, outPath);
  console.log(`✓ ${dest} done (${fmtBytes(got)})`);
}

async function main() {
  console.log(`Fetching bundled models into ${MODELS_DIR}`);
  await mkdir(PARAKEET_DIR, { recursive: true });
  await mkdir(FUNASR_DIR, { recursive: true });
  for (const file of FILES) {
    await download(file);
  }
  console.log('\nAll bundled models are ready.');
}

main().catch((err) => {
  console.error(`\nError: ${err.message}`);
  process.exit(1);
});
