CREATE TABLE uima_locks (
  key TEXT PRIMARY KEY,
  token UUID NOT NULL,
  -- By the database's clock.
  expires_at TIMESTAMPTZ NOT NULL
);
