import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';

/**
 * Request the macOS permissions a recording needs, at the moment it needs them.
 *
 * These used to be requested up front by the onboarding wizard's permissions step.
 * That wizard is gone (models ship bundled, so there was nothing to set up), but the
 * permissions still have to be asked for somewhere: microphone access self-prompts on
 * first capture, while Screen Recording — which ScreenCaptureKit requires for system
 * audio — does not reliably prompt and otherwise fails silently, capturing nothing.
 *
 * Asking here also gives the prompt context the wizard couldn't: the user has just
 * pressed record, so it is obvious why macOS is asking.
 */

export type RecordingMode = 'both' | 'system' | 'microphone';

async function isMacOS(): Promise<boolean> {
  try {
    const { platform } = await import('@tauri-apps/plugin-os');
    return platform() === 'macos';
  } catch {
    return typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac');
  }
}

/**
 * Ensure permissions for `mode` are granted.
 *
 * Returns false only when the recording could not produce any audio at all — i.e. the
 * single requested source was denied. A denied system source in "Mic + System" is
 * reported but does not block, since the microphone still records.
 */
export async function ensureRecordingPermissions(mode: RecordingMode): Promise<boolean> {
  if (!(await isMacOS())) return true;

  const needsMic = mode === 'both' || mode === 'microphone';
  const needsSystem = mode === 'both' || mode === 'system';

  let micGranted = true;
  let systemGranted = true;

  if (needsMic) {
    try {
      micGranted = await invoke<boolean>('trigger_microphone_permission');
    } catch (err) {
      console.error('[permissions] Microphone request failed:', err);
      micGranted = false;
    }
  }

  if (needsSystem) {
    try {
      // Verifies the captured stream is not silence, so a stale grant that no longer
      // works is reported as denied rather than producing an empty recording.
      systemGranted = await invoke<boolean>('trigger_system_audio_permission_command');
    } catch (err) {
      console.error('[permissions] System audio request failed:', err);
      systemGranted = false;
    }
  }

  const openSettings = {
    label: 'Open Settings',
    onClick: () => {
      invoke('open_system_settings').catch((err) =>
        console.error('[permissions] Failed to open System Settings:', err)
      );
    },
  };

  // Nothing could be captured — block rather than produce a silent recording.
  if ((needsMic && !micGranted && !needsSystem) || (needsSystem && !systemGranted && !needsMic)) {
    toast.error(needsMic ? 'Microphone access denied' : 'System audio access denied', {
      description: needsMic
        ? 'Grant microphone access to Meetily, then start recording again.'
        : 'Grant Screen Recording access to Meetily, then start recording again.',
      duration: 8000,
      action: openSettings,
    });
    return false;
  }

  // Partial denial in "Mic + System": warn, but let the usable source record.
  if (mode === 'both' && (!micGranted || !systemGranted)) {
    toast.warning(!systemGranted ? 'System audio unavailable' : 'Microphone unavailable', {
      description: !systemGranted
        ? 'Recording the microphone only. Grant Screen Recording access to capture system audio.'
        : 'Recording system audio only. Grant microphone access to capture your voice.',
      duration: 8000,
      action: openSettings,
    });
  }

  return true;
}
