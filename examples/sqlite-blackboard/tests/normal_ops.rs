/// 正常系テスト: DELETE + incremental_vacuum の動作確認
///
/// テスト1: `test_deleted_data_invisible_to_reader`
///   古いデータを DELETE した後、同じ SELECT クエリの結果が変わること。
///   latest_states は messages と独立しており、DELETE の影響を受けないこと。
///
/// テスト2: `test_vacuum_reduces_free_pages`
///   DELETE で増えた freelist が incremental_vacuum で解放され、
///   物理ページ数（page_count）が削減されること。
use rusqlite::params;
use sqlite_blackboard::db;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}

/// テスト用の participant / channel を登録し、channel_id を返す
fn setup_channel(conn: &rusqlite::Connection, guid: &str, topic: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO participants (guid, name) VALUES (?1, ?2)",
        params![guid, guid],
    )?;
    let p_id: i64 = conn.query_row(
        "SELECT id FROM participants WHERE guid = ?1",
        params![guid],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO channels (topic_name, participant_id) VALUES (?1, ?2)",
        params![topic, p_id],
    )?;
    conn.query_row(
        "SELECT id FROM channels WHERE topic_name = ?1 AND participant_id = ?2",
        params![topic, p_id],
        |r| r.get(0),
    )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// テスト1: DELETE したデータが Reader クエリから見えなくなること
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#[test]
fn test_deleted_data_invisible_to_reader() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("test.db");
    let conn = db::open_writer(&db_path)?;
    let ch_id = setup_channel(&conn, "guid-delete-test", "sensor/0")?;

    let now = now_nanos();
    // 10秒前のタイムスタンプ（= "古いデータ"）と 1秒前（= "新しいデータ"）を用意
    let old_ts = now - 10_000_000_000i64;
    let new_ts = now - 1_000_000_000i64;
    let data = vec![0u8; 256];

    // 古いデータ 100 件 + 新しいデータ 100 件を挿入
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    for i in 0..100i64 {
        conn.execute(
            "INSERT INTO messages (log_time, channel_id, data) VALUES (?1, ?2, ?3)",
            params![old_ts + i, ch_id, &data],
        )?;
    }
    for i in 0..100i64 {
        conn.execute(
            "INSERT INTO messages (log_time, channel_id, data) VALUES (?1, ?2, ?3)",
            params![new_ts + i, ch_id, &data],
        )?;
    }
    // latest_states に最新値を登録
    conn.execute(
        "INSERT OR REPLACE INTO latest_states (channel_id, timestamp, data_json) \
         VALUES (?1, ?2, '{}')",
        params![ch_id, new_ts + 99],
    )?;
    conn.execute_batch("COMMIT;")?;

    // cutoff: 5秒前 = 古いデータ(10s前)は対象外、新しいデータ(1s前)は対象内
    let cutoff_5s = now - 5_000_000_000i64;
    // cutoff: 15秒前 = 古い・新しいデータ両方が対象
    let cutoff_15s = now - 15_000_000_000i64;

    // ── DELETE 前 ─────────────────────────────────────────────
    let before_wide: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE log_time > ?1",
        params![cutoff_15s],
        |r| r.get(0),
    )?;
    assert_eq!(before_wide, 200, "DELETE前(cutoff=15s): 全200件が見えること");

    let before_narrow: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE log_time > ?1",
        params![cutoff_5s],
        |r| r.get(0),
    )?;
    assert_eq!(before_narrow, 100, "DELETE前(cutoff=5s): 直近100件のみ見えること");

    // ── DELETE 実行: 5秒より古いデータを削除 ───────────────────
    let deleted = conn.execute(
        "DELETE FROM messages WHERE log_time < ?1",
        params![cutoff_5s],
    )?;
    assert_eq!(deleted, 100, "DELETE: 100件削除されること");

    // ── DELETE 後 ─────────────────────────────────────────────
    // 同じ wide クエリ → 結果が変わること（削除が反映されていることの確認）
    let after_wide: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE log_time > ?1",
        params![cutoff_15s],
        |r| r.get(0),
    )?;
    assert_eq!(
        after_wide, 100,
        "DELETE後(cutoff=15s): 同じクエリの結果が 200→100 に変化すること"
    );

    // 新しいデータだけを対象にした narrow クエリ → 影響なし
    let after_narrow: i64 = conn.query_row(
        "SELECT count(*) FROM messages WHERE log_time > ?1",
        params![cutoff_5s],
        |r| r.get(0),
    )?;
    assert_eq!(
        after_narrow, 100,
        "DELETE後(cutoff=5s): 削除対象外のデータは変化なし"
    );

    // latest_states は messages の DELETE 対象外であること
    let ls_count: i64 = conn.query_row(
        "SELECT count(*) FROM latest_states WHERE channel_id = ?1",
        params![ch_id],
        |r| r.get(0),
    )?;
    assert_eq!(ls_count, 1, "latest_states は messages の DELETE の影響を受けないこと");

    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// テスト2: incremental_vacuum でフリーページが物理的に解放されること
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#[test]
fn test_vacuum_reduces_free_pages() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("test.db");
    let conn = db::open_writer(&db_path)?;
    let ch_id = setup_channel(&conn, "guid-vacuum-test", "sensor/0")?;

    // auto_vacuum が INCREMENTAL (=2) に設定されていることを確認
    // ※ journal_mode = WAL より先に設定しないと有効にならないため pragma 順序が重要
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    assert_eq!(
        auto_vacuum, 2,
        "auto_vacuum = INCREMENTAL (2) が設定されていること。\
         journal_mode = WAL より先に設定する必要がある"
    );

    let now = now_nanos();
    // 4KB × 200 件 ≒ 800KB → ページを十分に確保する
    let data = vec![0u8; 4096];

    conn.execute_batch("BEGIN IMMEDIATE;")?;
    for i in 0..200i64 {
        conn.execute(
            "INSERT INTO messages (log_time, channel_id, data) VALUES (?1, ?2, ?3)",
            params![now - 10_000_000_000i64 + i, ch_id, &data],
        )?;
    }
    conn.execute_batch("COMMIT;")?;

    // ── DELETE 前のページ情報 ──────────────────────────────────
    let page_count_before: i64 =
        conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let freelist_before: i64 =
        conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;

    // ── DELETE 実行 ────────────────────────────────────────────
    let deleted = conn.execute(
        "DELETE FROM messages WHERE channel_id = ?1",
        params![ch_id],
    )?;
    assert_eq!(deleted, 200, "DELETE: 200件削除されること");

    // DELETE 後: フリーページが増加しているはず
    let freelist_after_delete: i64 =
        conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    assert!(
        freelist_after_delete > freelist_before,
        "DELETE後: freelist_count が増加すること \
         (before={freelist_before}, after_delete={freelist_after_delete})"
    );

    // ── WAL フラッシュ → incremental_vacuum ───────────────────
    // WAL モードでは CHECKPOINT 後でないとページ移動が反映されない
    conn.execute_batch("PRAGMA wal_checkpoint(FULL);")?;
    conn.execute_batch("PRAGMA incremental_vacuum;")?;

    // VACUUM 後: フリーページが減少し、物理ページ数も削減されること
    let freelist_after_vacuum: i64 =
        conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let page_count_after: i64 =
        conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;

    assert!(
        freelist_after_vacuum < freelist_after_delete,
        "VACUUM後: freelist_count が減少すること \
         (after_delete={freelist_after_delete}, after_vacuum={freelist_after_vacuum})"
    );
    assert!(
        page_count_after < page_count_before,
        "VACUUM後: 物理ページ数が減少すること \
         (before={page_count_before}, after={page_count_after})"
    );

    Ok(())
}
