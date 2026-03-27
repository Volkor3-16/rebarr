INSERT OR IGNORE INTO Settings (key, value) VALUES ('auto_unmonitor_completed', 'false');

CREATE TABLE IF NOT EXISTS WebhookEndpoint (
    uuid        TEXT PRIMARY KEY,
    target_url  TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS WebhookEventFilter (
    webhook_id   TEXT NOT NULL REFERENCES WebhookEndpoint(uuid) ON DELETE CASCADE,
    task_type    TEXT NOT NULL,
    task_status  TEXT NOT NULL,
    PRIMARY KEY (webhook_id, task_type, task_status)
);

CREATE INDEX IF NOT EXISTS idx_webhook_filter_lookup
ON WebhookEventFilter(task_type, task_status);
