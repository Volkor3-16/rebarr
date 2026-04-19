-- Seed a default metadata rule that clears known aggregator uploader names from scanlator_group.
-- INSERT OR IGNORE: only runs if no metadata_rules key exists yet (i.e. first-time users).
-- Existing users with their own rules are unaffected.
INSERT OR IGNORE INTO Settings (key, value)
VALUES (
    'metadata_rules',
    '[{"id":"default-1","sort_order":0,"name":"Clear known aggregator uploader names","field":"title","action":"clear","pattern":"(?i)^(Kaos|1r0n|danke-empire|Ushi|\\(Ushi\\))$"}]'
);
