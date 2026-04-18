-- 1. 送信者管理
CREATE TABLE IF NOT EXISTS participants (
    id   INTEGER PRIMARY KEY,
    guid TEXT UNIQUE,
    name TEXT
);

-- 2. チャネル（トピック）管理
CREATE TABLE IF NOT EXISTS channels (
    id             INTEGER PRIMARY KEY,
    topic_name     TEXT,
    participant_id INTEGER,
    FOREIGN KEY(participant_id) REFERENCES participants(id),
    UNIQUE(topic_name, participant_id)
);

-- 3. メッセージ履歴（時系列）
CREATE TABLE IF NOT EXISTS messages (
    log_time   INTEGER,  -- ナノ秒
    channel_id INTEGER,
    data       BLOB,
    PRIMARY KEY (log_time, channel_id)
) WITHOUT ROWID;

-- 4. 最新値（ブラックボード）
CREATE TABLE IF NOT EXISTS latest_states (
    channel_id INTEGER PRIMARY KEY,
    timestamp  INTEGER,
    data_json  TEXT
);
