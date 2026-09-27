CREATE TABLE macro_migration_data (
    id INTEGER PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT INTO macro_migration_data (id, value) VALUES (1, 'migrated');
