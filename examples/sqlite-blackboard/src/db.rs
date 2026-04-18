use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

const SCHEMA_SQL: &str = include_str!("../schema.sql");

/// ライター用DB接続を開く（読み書き、スキーマ初期化込み）
pub fn open_writer(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    apply_writer_pragmas(&conn)?;
    apply_schema(&conn)?;
    Ok(conn)
}

/// リーダー用DB接続を開く（読み取り専用）
pub fn open_reader(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    apply_reader_pragmas(&conn)?;
    Ok(conn)
}

fn apply_writer_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        // auto_vacuum は DB 生成前（journal_mode より先）に設定しないと有効にならない
        "PRAGMA auto_vacuum = INCREMENTAL;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = OFF;
         PRAGMA mmap_size = 268435456;",
    )?;
    Ok(())
}

fn apply_reader_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA mmap_size = 268435456;",
    )?;
    Ok(())
}

fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}
