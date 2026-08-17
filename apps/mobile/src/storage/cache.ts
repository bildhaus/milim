import {open} from '@op-engineering/op-sqlite';
import type {SavedHost, TimelinePageV1} from '../control/types';
import {
  normalizeModelPickerPreferences,
  type MobileModelPickerPreferences,
} from '../modelPicker';

const db = open({name: 'milim-mobile.sqlite'});
let ready: Promise<void> | null = null;

function initialize(): Promise<void> {
  if (!ready) {
    ready = db
      .execute('PRAGMA journal_mode = WAL')
      .then(() =>
        db.executeBatch([
        [
          `CREATE TABLE IF NOT EXISTS hosts (
            host_id TEXT PRIMARY KEY,
            metadata_json TEXT NOT NULL,
            last_used_at INTEGER NOT NULL
          )`,
        ],
        [
          `CREATE TABLE IF NOT EXISTS timeline_tails (
            host_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            epoch TEXT NOT NULL,
            last_seq INTEGER,
            page_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(host_id, thread_id)
          )`,
        ],
        [
          `CREATE TABLE IF NOT EXISTS drafts (
            host_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            text TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(host_id, thread_id)
          )`,
        ],
        [
          `CREATE TABLE IF NOT EXISTS temporary_files (
            uri TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL
          )`,
        ],
        [
          `CREATE TABLE IF NOT EXISTS model_picker_preferences (
            host_id TEXT PRIMARY KEY,
            preferences_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
          )`,
        ],
        ]),
      )
      .then(() => undefined);
  }
  return ready;
}

export async function listHosts(): Promise<SavedHost[]> {
  await initialize();
  const result = await db.execute('SELECT metadata_json FROM hosts ORDER BY last_used_at DESC');
  return result.rows.map(row => JSON.parse(String(row.metadata_json)) as SavedHost);
}

export async function saveHost(host: SavedHost): Promise<void> {
  await initialize();
  await db.execute(
    `INSERT INTO hosts(host_id, metadata_json, last_used_at) VALUES(?, ?, ?)
     ON CONFLICT(host_id) DO UPDATE SET metadata_json = excluded.metadata_json,
       last_used_at = excluded.last_used_at`,
    [host.hostId, JSON.stringify(host), Date.now()],
  );
}

export async function removeHost(hostId: string): Promise<void> {
  await initialize();
  await db.executeBatch([
    ['DELETE FROM timeline_tails WHERE host_id = ?', [hostId]],
    ['DELETE FROM drafts WHERE host_id = ?', [hostId]],
    ['DELETE FROM model_picker_preferences WHERE host_id = ?', [hostId]],
    ['DELETE FROM hosts WHERE host_id = ?', [hostId]],
  ]);
}

export async function saveTimelineTail(hostId: string, page: TimelinePageV1): Promise<void> {
  await initialize();
  await db.execute(
    `INSERT INTO timeline_tails(host_id, thread_id, epoch, last_seq, page_json, updated_at)
     VALUES(?, ?, ?, ?, ?, ?)
     ON CONFLICT(host_id, thread_id) DO UPDATE SET epoch = excluded.epoch,
       last_seq = excluded.last_seq, page_json = excluded.page_json,
       updated_at = excluded.updated_at`,
    [hostId, page.thread_id, page.epoch, page.last_seq, JSON.stringify(page), Date.now()],
  );
}

export async function readTimelineTail(
  hostId: string,
  threadId: string,
): Promise<TimelinePageV1 | null> {
  await initialize();
  const result = await db.execute(
    'SELECT page_json FROM timeline_tails WHERE host_id = ? AND thread_id = ?',
    [hostId, threadId],
  );
  const raw = result.rows[0]?.page_json;
  return raw ? (JSON.parse(String(raw)) as TimelinePageV1) : null;
}

export async function saveDraft(hostId: string, threadId: string, text: string): Promise<void> {
  await initialize();
  if (!text) {
    await db.execute('DELETE FROM drafts WHERE host_id = ? AND thread_id = ?', [hostId, threadId]);
    return;
  }
  await db.execute(
    `INSERT INTO drafts(host_id, thread_id, text, updated_at) VALUES(?, ?, ?, ?)
     ON CONFLICT(host_id, thread_id) DO UPDATE SET text = excluded.text,
       updated_at = excluded.updated_at`,
    [hostId, threadId, text, Date.now()],
  );
}

export async function readDraft(hostId: string, threadId: string): Promise<string> {
  await initialize();
  const result = await db.execute(
    'SELECT text FROM drafts WHERE host_id = ? AND thread_id = ?',
    [hostId, threadId],
  );
  return String(result.rows[0]?.text ?? '');
}

export async function trackTemporaryFile(uri: string): Promise<void> {
  await initialize();
  await db.execute('INSERT OR REPLACE INTO temporary_files(uri, created_at) VALUES(?, ?)', [
    uri,
    Date.now(),
  ]);
}

export async function forgetTemporaryFile(uri: string): Promise<void> {
  await initialize();
  await db.execute('DELETE FROM temporary_files WHERE uri = ?', [uri]);
}

export async function staleTemporaryFiles(maxAgeMs = 24 * 60 * 60 * 1000): Promise<string[]> {
  await initialize();
  const result = await db.execute(
    'SELECT uri FROM temporary_files WHERE created_at < ? ORDER BY created_at ASC LIMIT 50',
    [Date.now() - maxAgeMs],
  );
  return result.rows.map(row => String(row.uri));
}

export async function readModelPickerPreferences(
  hostId: string,
): Promise<MobileModelPickerPreferences> {
  await initialize();
  const result = await db.execute(
    'SELECT preferences_json FROM model_picker_preferences WHERE host_id = ?',
    [hostId],
  );
  const raw = result.rows[0]?.preferences_json;
  if (!raw) return normalizeModelPickerPreferences(null);
  try {
    return normalizeModelPickerPreferences(JSON.parse(String(raw)));
  } catch {
    return normalizeModelPickerPreferences(null);
  }
}

export async function saveModelPickerPreferences(
  hostId: string,
  preferences: MobileModelPickerPreferences,
): Promise<void> {
  await initialize();
  const normalized = normalizeModelPickerPreferences(preferences);
  await db.execute(
    `INSERT INTO model_picker_preferences(host_id, preferences_json, updated_at) VALUES(?, ?, ?)
     ON CONFLICT(host_id) DO UPDATE SET preferences_json = excluded.preferences_json,
       updated_at = excluded.updated_at`,
    [hostId, JSON.stringify(normalized), Date.now()],
  );
}
