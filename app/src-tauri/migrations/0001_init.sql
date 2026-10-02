-- 0001: initial schema, copied from docs/05-data-model.md. Never edit once released (AGENTS.md rule 9).

CREATE TABLE people (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  email TEXT,
  is_self INTEGER NOT NULL DEFAULT 0,
  voice_embedding BLOB,                 -- opt-in only (NFR-16)
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE meetings (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  source_app TEXT NOT NULL,             -- zoom | meet | slack | teams | discord | in_person | import | other
  template TEXT NOT NULL DEFAULT 'general',
  started_at INTEGER NOT NULL,
  ended_at INTEGER,
  duration_s INTEGER,
  scheduled_duration_s INTEGER,         -- from calendar, for overrun
  status TEXT NOT NULL,                 -- recording | processing | ready | failed
  audio_dir TEXT NOT NULL,              -- folder with mic/ and sys/ chunk files
  audio_deleted_at INTEGER,
  calendar_event_id TEXT,
  language TEXT DEFAULT 'en',
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE speakers (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  person_id TEXT REFERENCES people(id) ON DELETE SET NULL,
  label TEXT NOT NULL,                  -- "Speaker 1" until renamed
  is_me INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE segments (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  speaker_id TEXT REFERENCES speakers(id) ON DELETE SET NULL,
  track TEXT NOT NULL,                  -- mic | sys
  start_ms INTEGER NOT NULL,
  end_ms INTEGER NOT NULL,
  text TEXT NOT NULL,
  words TEXT,                           -- JSON [{w, s, e, conf}]
  edited INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX idx_segments_meeting_time ON segments(meeting_id, start_ms);

CREATE TABLE reports (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL UNIQUE REFERENCES meetings(id) ON DELETE CASCADE,
  summary_md TEXT NOT NULL,
  key_points TEXT NOT NULL,             -- JSON
  decisions TEXT NOT NULL,              -- JSON
  open_questions TEXT NOT NULL,         -- JSON
  suggestions TEXT NOT NULL,            -- JSON (FR-5.5)
  chapters TEXT,                        -- JSON (FR-4.7)
  follow_up_email TEXT,
  model_used TEXT NOT NULL,
  cost_usd REAL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE action_items (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  owner_person_id TEXT REFERENCES people(id) ON DELETE SET NULL,
  owner_label TEXT,                     -- raw name if not matched
  task TEXT NOT NULL,
  due_date TEXT,                        -- YYYY-MM-DD if stated
  done INTEGER NOT NULL DEFAULT 0,
  segment_id TEXT REFERENCES segments(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE scores (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                   -- engagement | value | my_performance | productivity | metric:<name>
  value REAL NOT NULL,                  -- 0–100 for scores; raw for metrics
  evidence TEXT NOT NULL,               -- JSON {metrics:{}, segment_ids:[], rationale:""}
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  UNIQUE(meeting_id, kind)
);

CREATE TABLE highlights (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  at_ms INTEGER NOT NULL,
  note TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE tags (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE meeting_tags (meeting_id TEXT REFERENCES meetings(id) ON DELETE CASCADE, tag_id TEXT REFERENCES tags(id) ON DELETE CASCADE, PRIMARY KEY (meeting_id, tag_id));

CREATE TABLE jobs (
  id TEXT PRIMARY KEY,
  meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
  step TEXT NOT NULL,                   -- transcribe_mic | transcribe_system | merge | metrics | analyze | index | notify
  status TEXT NOT NULL,                 -- queued | running | done | failed
  attempts INTEGER NOT NULL DEFAULT 0,
  next_run_at INTEGER,
  error TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX idx_jobs_status ON jobs(status, next_run_at);

CREATE TABLE app_rules (               -- FR-1.7
  source_app TEXT PRIMARY KEY,
  rule TEXT NOT NULL,                   -- ask | always | never
  updated_at INTEGER NOT NULL
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL); -- never API keys

CREATE VIRTUAL TABLE segments_fts USING fts5(text, content='segments', content_rowid='rowid');
-- Phase 3: CREATE VIRTUAL TABLE segment_embeddings USING vec0(segment_id TEXT PRIMARY KEY, embedding float[768]);
