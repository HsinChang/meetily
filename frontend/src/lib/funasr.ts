// Types and invoke wrappers for the Fun-ASR (FunAudioLLM) transcription engine.
//
// Fun-ASR-Nano is Chinese-first ASR, used where Whisper degrades on noisy Mandarin.
// Inference runs in the funasr-helper sidecar; see funasr-helper/README.md.

import { invoke } from '@tauri-apps/api/core';

export type FunAsrModelStatus =
  | 'Available'
  | 'Missing'
  | { Downloading: { progress: number } }
  | { Error: string }
  | { Corrupted: { file_size: number; expected_min_size: number } };

export interface FunAsrModelInfo {
  name: string;
  path: string;
  size_mb: number;
  accuracy: string;
  speed: string;
  status: FunAsrModelStatus;
  description?: string;
}

export interface FunAsrDownloadProgress {
  modelName: string;
  progress: number;
  downloaded_mb: number;
  total_mb: number;
  speed_mbps: number;
  status: 'downloading' | 'completed';
}

export interface FunAsrModelDisplayInfo {
  friendlyName: string;
  icon: string;
  tagline: string;
  recommended?: boolean;
}

/** Keep model names in sync with FUNASR_MODEL_CATALOG in src-tauri/src/config.rs */
export const FUNASR_MODEL_DISPLAY_CONFIG: Record<string, FunAsrModelDisplayInfo> = {
  'fun-asr-nano-2512-q8': {
    friendlyName: 'Fun-ASR Nano',
    icon: '🇨🇳',
    tagline: 'Chinese, 7 dialects & 26 accents • ~20x real time',
    recommended: true,
  },
};

export function getFunAsrModelDisplayInfo(modelName: string): FunAsrModelDisplayInfo | undefined {
  return FUNASR_MODEL_DISPLAY_CONFIG[modelName];
}

export function getFunAsrModelDisplayName(modelName: string): string {
  return FUNASR_MODEL_DISPLAY_CONFIG[modelName]?.friendlyName ?? modelName;
}

export function formatModelSize(sizeMb: number): string {
  return sizeMb >= 1024 ? `${(sizeMb / 1024).toFixed(1)} GB` : `${sizeMb} MB`;
}

export function isModelAvailable(status: FunAsrModelStatus): boolean {
  return status === 'Available';
}

export function isModelDownloading(status: FunAsrModelStatus): boolean {
  return typeof status === 'object' && 'Downloading' in status;
}

export function getDownloadProgress(status: FunAsrModelStatus): number {
  return typeof status === 'object' && 'Downloading' in status ? status.Downloading.progress : 0;
}

export function getModelError(status: FunAsrModelStatus): string | null {
  if (typeof status === 'object' && 'Error' in status) return status.Error;
  if (typeof status === 'object' && 'Corrupted' in status) {
    return `File is incomplete (${status.Corrupted.file_size} bytes). Re-download to fix.`;
  }
  return null;
}

export const FunAsrAPI = {
  init: () => invoke<void>('funasr_init'),
  getAvailableModels: () => invoke<FunAsrModelInfo[]>('funasr_get_available_models'),
  loadModel: (modelName: string) => invoke<void>('funasr_load_model', { modelName }),
  unloadModel: () => invoke<boolean>('funasr_unload_model'),
  getCurrentModel: () => invoke<string | null>('funasr_get_current_model'),
  isModelLoaded: () => invoke<boolean>('funasr_is_model_loaded'),
  hasAvailableModels: () => invoke<boolean>('funasr_has_available_models'),
  getModelsDirectory: () => invoke<string>('funasr_get_models_directory'),
  downloadModel: (modelName: string) => invoke<void>('funasr_download_model', { modelName }),
  cancelDownload: (modelName: string) => invoke<void>('funasr_cancel_download', { modelName }),
  deleteModel: (modelName: string) => invoke<string>('funasr_delete_model', { modelName }),
};
