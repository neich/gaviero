/// SQL statements for the memory store schema.

pub const CREATE_MEMORIES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS memories (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace   TEXT    NOT NULL,
    key         TEXT    NOT NULL,
    content     TEXT    NOT NULL,
    embedding   BLOB,
    model_id    TEXT,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    metadata    TEXT,
    UNIQUE(namespace, key)
)";

pub const CREATE_NAMESPACE_INDEX: &str = "
CREATE INDEX IF NOT EXISTS idx_memories_namespace ON memories(namespace)";

pub const CREATE_KEY_INDEX: &str = "
CREATE INDEX IF NOT EXISTS idx_memories_ns_key ON memories(namespace, key)";
