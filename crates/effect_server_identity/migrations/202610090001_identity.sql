-- Effect Identity persistence schema v1. Applied through the shared SQLx journal.
-- No byte/schema compatibility with native Identity Turso persistence is implied.
CREATE TABLE effect_identity_browser_contexts (
    id TEXT PRIMARY KEY NOT NULL,
    expires_at INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX idx_effect_identity_browser_contexts_expiry ON effect_identity_browser_contexts(expires_at);
CREATE TABLE effect_identity_users (
    id TEXT PRIMARY KEY NOT NULL,
    aggregate_revision INTEGER NOT NULL DEFAULT 0 CHECK(typeof(aggregate_revision)='integer' AND aggregate_revision >= 0),
    normalized_user_name TEXT NOT NULL UNIQUE,
    normalized_email TEXT,
    concurrency_stamp TEXT NOT NULL,
    payload TEXT NOT NULL
);

CREATE INDEX idx_effect_identity_users_normalized_user_name
    ON effect_identity_users(normalized_user_name);

CREATE INDEX idx_effect_identity_users_normalized_email
    ON effect_identity_users(normalized_email);

CREATE TABLE effect_identity_protected_users (
    user_id TEXT PRIMARY KEY NOT NULL,
    require_unique INTEGER NOT NULL CHECK(require_unique IN (0, 1)),
    payload TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);
CREATE TABLE effect_identity_protected_indexes (
    user_id TEXT NOT NULL,
    field TEXT NOT NULL,
    key_id TEXT NOT NULL,
    digest TEXT NOT NULL,
    PRIMARY KEY (user_id, field, key_id),
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);
CREATE INDEX idx_effect_identity_protected_lookup
    ON effect_identity_protected_indexes(field, key_id, digest);

CREATE TABLE effect_identity_user_email_policy (
    user_id TEXT PRIMARY KEY NOT NULL,
    require_unique INTEGER NOT NULL CHECK(require_unique IN (0, 1)),
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);

CREATE TABLE effect_identity_roles (
    id TEXT PRIMARY KEY NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE,
    concurrency_stamp TEXT NOT NULL,
    payload TEXT NOT NULL
);

CREATE INDEX idx_effect_identity_roles_normalized_name
    ON effect_identity_roles(normalized_name);

CREATE TABLE effect_identity_user_roles (
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    PRIMARY KEY (user_id, role_id),
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE,
    FOREIGN KEY (role_id) REFERENCES effect_identity_roles(id) ON DELETE CASCADE
);

CREATE INDEX idx_effect_identity_user_roles_role_id
    ON effect_identity_user_roles(role_id);

CREATE TABLE effect_identity_user_claims (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    claim_type TEXT NOT NULL,
    claim_value TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);

CREATE INDEX idx_effect_identity_user_claims_claim
    ON effect_identity_user_claims(claim_type, claim_value);

CREATE INDEX idx_effect_identity_user_claims_user_id
    ON effect_identity_user_claims(user_id);

CREATE TABLE effect_identity_user_logins (
    user_id TEXT NOT NULL,
    login_provider TEXT NOT NULL,
    provider_key TEXT NOT NULL,
    provider_display_name TEXT,
    PRIMARY KEY (login_provider, provider_key),
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);

CREATE INDEX idx_effect_identity_user_logins_user_id
    ON effect_identity_user_logins(user_id);

CREATE TABLE effect_identity_user_tokens (
    user_id TEXT NOT NULL,
    login_provider TEXT NOT NULL,
    name TEXT NOT NULL,
    value TEXT,
    PRIMARY KEY (user_id, login_provider, name),
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);

CREATE TABLE effect_identity_user_recovery_code_hashes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);

CREATE INDEX idx_effect_identity_user_recovery_code_hashes_user_id
    ON effect_identity_user_recovery_code_hashes(user_id);
-- A deterministic native recovery hash can represent only one usable secret.
CREATE UNIQUE INDEX idx_effect_identity_recovery_hash_owner_unique
    ON effect_identity_user_recovery_code_hashes(user_id,code_hash);

CREATE TABLE effect_identity_user_passkeys (
    user_id TEXT NOT NULL,
    credential_id BLOB PRIMARY KEY NOT NULL,
    payload TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES effect_identity_users(id) ON DELETE CASCADE
);

CREATE INDEX idx_effect_identity_user_passkeys_user_id
    ON effect_identity_user_passkeys(user_id);

CREATE TABLE effect_identity_passkey_challenges (
    challenge BLOB PRIMARY KEY NOT NULL,
    purpose TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    payload TEXT NOT NULL
);

CREATE INDEX idx_effect_identity_passkey_challenges_expires_at
    ON effect_identity_passkey_challenges(expires_at);

CREATE TABLE effect_identity_role_claims (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    role_id TEXT NOT NULL,
    claim_type TEXT NOT NULL,
    claim_value TEXT NOT NULL,
    FOREIGN KEY (role_id) REFERENCES effect_identity_roles(id) ON DELETE CASCADE
);

CREATE INDEX idx_effect_identity_role_claims_role_id
    ON effect_identity_role_claims(role_id);

-- Extension records are separate from public account views and native credentials.
CREATE TABLE effect_identity_records (
    category TEXT NOT NULL,
    owner TEXT NOT NULL,
    record_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    codec_version INTEGER NOT NULL,
    expires_at INTEGER,
    payload BLOB NOT NULL,
    PRIMARY KEY(category, owner, record_id)
);
CREATE INDEX effect_identity_records_expiry ON effect_identity_records(category, expires_at);
CREATE TABLE effect_identity_audit (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    owner TEXT NOT NULL,
    event_id TEXT NOT NULL UNIQUE,
    codec_version INTEGER NOT NULL,
    payload BLOB NOT NULL
);
CREATE INDEX effect_identity_audit_owner ON effect_identity_audit(owner, sequence);

-- Aggregate revision v1 is a signed 64-bit logical sequence, separate from native
-- public account concurrency stamps. Every granular child mutation participates.
CREATE TRIGGER effect_identity_user_aggregate_update AFTER UPDATE OF payload,normalized_user_name,normalized_email ON effect_identity_users
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.id; END;
CREATE TRIGGER effect_identity_user_roles_insert AFTER INSERT ON effect_identity_user_roles
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_roles_update AFTER UPDATE ON effect_identity_user_roles
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_roles_delete AFTER DELETE ON effect_identity_user_roles
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_user_claims_insert AFTER INSERT ON effect_identity_user_claims
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_claims_update AFTER UPDATE ON effect_identity_user_claims
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_claims_delete AFTER DELETE ON effect_identity_user_claims
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_user_logins_insert AFTER INSERT ON effect_identity_user_logins
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_logins_update AFTER UPDATE ON effect_identity_user_logins
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_logins_delete AFTER DELETE ON effect_identity_user_logins
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_user_tokens_insert AFTER INSERT ON effect_identity_user_tokens
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_tokens_update AFTER UPDATE ON effect_identity_user_tokens
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_tokens_delete AFTER DELETE ON effect_identity_user_tokens
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_user_recovery_code_hashes_insert AFTER INSERT ON effect_identity_user_recovery_code_hashes
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_recovery_code_hashes_update AFTER UPDATE ON effect_identity_user_recovery_code_hashes
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_recovery_code_hashes_delete AFTER DELETE ON effect_identity_user_recovery_code_hashes
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_user_passkeys_insert AFTER INSERT ON effect_identity_user_passkeys
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_passkeys_update AFTER UPDATE ON effect_identity_user_passkeys
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_passkeys_delete AFTER DELETE ON effect_identity_user_passkeys
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_protected_users_insert AFTER INSERT ON effect_identity_protected_users
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_protected_users_update AFTER UPDATE ON effect_identity_protected_users
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_protected_users_delete AFTER DELETE ON effect_identity_protected_users
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_protected_indexes_insert AFTER INSERT ON effect_identity_protected_indexes
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_protected_indexes_update AFTER UPDATE ON effect_identity_protected_indexes
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_protected_indexes_delete AFTER DELETE ON effect_identity_protected_indexes
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
CREATE TRIGGER effect_identity_user_email_policy_insert AFTER INSERT ON effect_identity_user_email_policy
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_email_policy_update AFTER UPDATE ON effect_identity_user_email_policy
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=NEW.user_id; END;
CREATE TRIGGER effect_identity_user_email_policy_delete AFTER DELETE ON effect_identity_user_email_policy
BEGIN UPDATE effect_identity_users SET aggregate_revision=aggregate_revision+1 WHERE id=OLD.user_id; END;
