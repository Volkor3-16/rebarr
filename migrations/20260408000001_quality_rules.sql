-- Quality rules engine: replaces the fixed tier system with user-configurable scoring rules.
-- Each rule evaluates a set of conditions against a chapter; if all conditions match, the
-- rule's score is added to the chapter's total score. Higher score = preferred source.
CREATE TABLE QualityRules (
    id          TEXT PRIMARY KEY,
    sort_order  INTEGER NOT NULL,
    name        TEXT NOT NULL,
    score       INTEGER NOT NULL,
    -- JSON array of condition objects: [{field, op, value?, negate?}]
    -- Supported fields (v1): scanlator_group, provider_name, language, title, released_at
    -- Supported ops: eq, contains, regex, present, not_present
    conditions  TEXT NOT NULL DEFAULT '[]'
);

-- Default rules that replicate the current hardcoded tier behaviour.
INSERT INTO QualityRules (id, sort_order, name, score, conditions) VALUES
    ('00000000-0000-0000-0000-000000000001', 10,  'Local',            10000, '[{"field":"provider_name","op":"eq","value":"Local"}]'),
    ('00000000-0000-0000-0000-000000000002', 20,  'Official',          8000, '[{"field":"scanlator_group","op":"regex","value":"(?i)^official[.!?]?$"}]'),
    ('00000000-0000-0000-0000-000000000080', 80,  'Has chapter title',   20, '[{"field":"title","op":"present"}]'),
    ('00000000-0000-0000-0000-000000000085', 85,  'Has release date',    10, '[{"field":"released_at","op":"present"}]'),
    ('00000000-0000-0000-0000-000000000090', 90,  'No scanlator group', -100, '[{"field":"scanlator_group","op":"present","negate":true}]');

-- Migrate existing TrustedGroup entries as quality rules (score=500, sort_order starts at 30).
-- Each gets a deterministic ID based on its row number so repeated migrations are idempotent.
INSERT OR IGNORE INTO QualityRules (id, sort_order, name, score, conditions)
SELECT
    lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' ||
        substr(lower(hex(randomblob(2))),2) || '-' ||
        substr('89ab', abs(random()) % 4 + 1, 1) ||
        substr(lower(hex(randomblob(2))),2) || '-' || lower(hex(randomblob(6))) AS id,
    30 + (row_number() OVER (ORDER BY name COLLATE NOCASE)) * 1 AS sort_order,
    'Trusted: ' || name AS name,
    500 AS score,
    json_array(json_object('field', 'scanlator_group', 'op', 'eq', 'value', name)) AS conditions
FROM TrustedGroup;
