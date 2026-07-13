-- Migration: Add translation field for real-time Chinese translation
-- Stores the translated (Simplified Chinese) text for a transcript segment so it
-- persists into the completed meeting view and survives re-transcription.

ALTER TABLE transcripts ADD COLUMN translation TEXT;
