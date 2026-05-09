import { useCallback, useEffect } from "react";
import { getSoundEngine, preloadSoundEngine, type SoundName } from "@/lib/sound-engine";

const SOUND_ENABLED_STORAGE_KEY = "asterel-sound-enabled";
const SOUND_VOLUME_STORAGE_KEY = "asterel-sound-volume";
const DEFAULT_SOUND_ENABLED = true;
const DEFAULT_SOUND_VOLUME = 0.3;

export interface UseSoundResult {
  play: (name: SoundName) => void;
}

function hasWindow(): boolean {
  return typeof window !== "undefined";
}

function isReducedMotionPreferred(): boolean {
  if (!hasWindow() || typeof window.matchMedia !== "function") {
    return false;
  }

  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function getSoundEnabled(): boolean {
  if (!hasWindow()) {
    return DEFAULT_SOUND_ENABLED;
  }

  try {
    const stored = window.localStorage.getItem(SOUND_ENABLED_STORAGE_KEY);
    if (stored == null) {
      return DEFAULT_SOUND_ENABLED;
    }

    return stored !== "false";
  } catch {
    return DEFAULT_SOUND_ENABLED;
  }
}

function getSoundVolume(): number {
  if (!hasWindow()) {
    return DEFAULT_SOUND_VOLUME;
  }

  try {
    const stored = window.localStorage.getItem(SOUND_VOLUME_STORAGE_KEY);
    if (stored == null) {
      return DEFAULT_SOUND_VOLUME;
    }

    const parsed = Number(stored);
    if (!Number.isFinite(parsed)) {
      return DEFAULT_SOUND_VOLUME;
    }

    return Math.min(Math.max(parsed, 0), 0.99);
  } catch {
    return DEFAULT_SOUND_VOLUME;
  }
}

export function useSound(): UseSoundResult {
  useEffect(() => {
    preloadSoundEngine();
  }, []);

  const play = useCallback((name: SoundName) => {
    if (!getSoundEnabled() || isReducedMotionPreferred()) {
      return;
    }

    void getSoundEngine().play(name, getSoundVolume());
  }, []);

  return { play };
}

export type { SoundName };
